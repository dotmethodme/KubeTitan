//! Transitions on the app's own state: the current view, the namespace filter,
//! the selection, the drawer and the status line. Nothing here touches the
//! cluster — that is [`super::watch`].

use super::*;

impl App {
    pub fn set_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        let keep_query =
            matches!(view, View::Resource(_)) && matches!(self.view, View::Resource(_));
        self.view = view;
        if !keep_query {
            self.query.clear();
        }
        self.close_drawer();
        let next_kind = match &self.view {
            View::Resource(id) => Some(*id),
            _ => None,
        };
        if let Some(id) = next_kind {
            self.restart_informer(id);
        }
    }

    /// Namespace selector changed: every store is namespace-scoped.
    pub fn on_namespace_changed(&mut self) {
        self.close_drawer();
        if let View::Resource(id) = self.view {
            self.restart_informer(id);
        }
        self.restart_events();
        self.restart_apps();
    }

    /// Switching context moves every watcher: they all hold the old client, and
    /// the stores hold the old cluster's objects. Errors are returned so the
    /// agent control channel can report them; the UI also shows them.
    pub fn switch_context(&mut self, name: String) -> Result<(), String> {
        if name == self.context {
            return Ok(());
        }
        let client = match self.rt.block_on(cluster::connect(Self::context_arg(&name))) {
            Ok(client) => client,
            Err(e) => {
                let message = format!("could not switch to `{name}`: {e}");
                self.flash_error(message.clone());
                return Err(message);
            }
        };
        self.client = client;
        self.context = name;
        self.contexts = cluster::context_names();
        self.restart_all_watchers();
        Ok(())
    }

    /// The context name as `cluster::connect` wants it: the in-cluster account
    /// is kube's own default. Takes the name explicitly, so a switch can never
    /// connect with the context it is leaving.
    fn context_arg(name: &str) -> Option<&str> {
        if name == "in-cluster" {
            None
        } else {
            Some(name)
        }
    }

    /// Rebuild every watcher against the client we hold, and drop what came from
    /// the old cluster. Shared by switching context and by Reconnect, because a
    /// watcher that died and a cluster that was swapped both need exactly this.
    fn restart_all_watchers(&mut self) {
        // nothing in hand belongs to the new cluster
        self.objects.clear();
        self.pods.clear();
        self.metrics.clear();
        self.events.clear();
        self.apps.clear();
        self.namespaces.clear();
        self.close_drawer();
        self.selection = None;
        self.yaml = None;
        self.save_settings();
        // the pods store is created on first use, so drop it and let it return —
        // aborting first: an informer left running would keep failing against a
        // cluster we have already left, and keep saying so
        if let Some(task) = self.pods_task.take() {
            task.abort();
        }
        if let View::Resource(id) = self.view {
            self.restart_informer(id);
        }
        self.restart_metrics();
        self.restart_events();
        self.restart_apps();
        self.ensure_pods_store();
        self.start_discovery();
        cluster::list_namespaces(
            &self.rt,
            self.client.clone(),
            forward_namespaces(self.tx.clone()),
        );
    }

    /// The banner's way out of a failure: a fresh client for the context we are
    /// already on — the answer to an expired token or a network that came back —
    /// and every watcher rebuilt.
    pub fn reconnect(&mut self) {
        match self
            .rt
            .block_on(cluster::connect(Self::context_arg(&self.context)))
        {
            Ok(client) => {
                self.client = client;
                self.restart_all_watchers();
                self.flash_status("reconnected".to_string());
            }
            Err(e) => self.flash_error(format!("still cannot reach the cluster: {e}")),
        }
    }

    /// Take the recovery the current notice offers, if it offers one.
    pub fn apply_notice_fix(&mut self) {
        let Some(fix) = self.notice.as_ref().and_then(|notice| notice.fix) else {
            return;
        };
        self.notice = None;
        match fix {
            notice::Fix::Reconnect => self.reconnect(),
        }
    }

    /// Refresh button: re-establish the watch (cheap: the apiserver replays).
    pub fn refresh_view(&mut self) {
        if let View::Resource(id) = self.view {
            self.restart_informer(id);
        }
    }

    pub(crate) fn loading_elapsed(&self) -> Duration {
        self.load_started.elapsed()
    }

    // -----------------------------------------------------------------------
    // selection + drawer
    // -----------------------------------------------------------------------

    pub fn select(&mut self, kind: &'static Kind, key: &str) {
        let Some(object) = self.objects.get(key).cloned() else {
            return;
        };

        self.stop_logs();
        self.logs.clear();
        self.yaml = None;
        self.cancel_yaml_edit();
        self.cancel_data_edit();
        self.pending = None;
        // logs default to the pod's first container, like Vue's selectedContainer
        self.log_container = cluster::pod_containers(&object).first().cloned();
        self.selection = Some(Selection {
            kind,
            namespace: object.metadata.namespace.clone().unwrap_or_default(),
            name: object.metadata.name.clone().unwrap_or_default(),
            object: Arc::new(object),
        });
        self.restart_object_events();
        // usage charts come from Prometheus, for pods and their workloads
        self.restart_timeline();
        self.start_logs();
        // The drawer's tab is remembered from resource to resource — switch to
        // Logs, open another pod, and you are still in Logs — so what the tab
        // shows has to be fetched for the *new* object. A remembered YAML tab
        // would otherwise open on the previous object's YAML.
        if self.drawer_tab == DrawerTab::Yaml {
            self.fetch_yaml();
        }
    }

    pub fn close_drawer(&mut self) {
        self.stop_logs();
        self.stop_object_events();
        self.stop_timeline();
        self.selection = None;
        self.logs.clear();
        self.yaml = None;
        self.cancel_yaml_edit();
        self.cancel_data_edit();
        self.pending = None;
    }

    /// Point the log pane at a container and restart the stream. The General
    /// tab's per-container Logs links come through here.
    pub fn select_log_container(&mut self, container: String) {
        self.log_container = Some(container);
        self.stop_logs();
        self.logs.clear();
        self.start_logs();
    }

    /// Turn line timestamps on or off. They are a property of the request, not
    /// of drawing — the apiserver stamps a line only if the stream asked it to —
    /// so this restarts the stream, the way switching containers does.
    pub fn set_log_timestamps(&mut self, on: bool) {
        if self.log_timestamps == on {
            return;
        }
        self.log_timestamps = on;
        self.stop_logs();
        self.logs.clear();
        self.start_logs();
    }

    pub fn set_drawer_tab(&mut self, tab: DrawerTab) {
        if self.drawer_tab == tab {
            return;
        }
        self.drawer_tab = tab;
        match tab {
            DrawerTab::Yaml => self.fetch_yaml(),
            DrawerTab::Logs if self.logs.is_empty() => {
                self.start_logs();
            }
            _ => {}
        }
    }

    /// Shows a message in the header status line for a few seconds.
    /// A confirmation: six seconds, then gone.
    pub fn flash_status(&mut self, text: String) {
        self.notice = Some(Notice::info(text));
    }

    /// A failure: it waits for the user, because a message that removes itself
    /// is a message that removes the chance to do something about it.
    pub fn flash_error(&mut self, text: String) {
        self.notice = Some(Notice::error(text));
    }

    /// Put the banner away.
    pub fn dismiss_notice(&mut self) {
        self.notice = None;
    }

    // -----------------------------------------------------------------------
    // palette plumbing
    // -----------------------------------------------------------------------

    pub fn switch_namespace(&mut self, ns: String) {
        if self.namespace == ns {
            return;
        }
        self.namespace = ns;
        self.save_settings();
        self.on_namespace_changed();
    }

    /// Persist the preferences that outlive the process (namespace, context,
    /// drawer width).
    pub(crate) fn save_settings(&mut self) {
        let settings = settings::Settings {
            namespace: self.namespace.clone(),
            context: Some(self.context.clone()),
            drawer_width: Some(self.drawer_width),
        };
        if let Err(e) = settings::save(&settings) {
            self.flash_status(format!("Could not save settings: {e}"));
        }
    }
}
