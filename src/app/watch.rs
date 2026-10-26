//! Cluster wiring: every task that talks to the apiserver, and the channel
//! adapters that turn their output into [`Msg`]s for the UI thread.

use super::*;

impl App {
    // -----------------------------------------------------------------------
    // informers
    // -----------------------------------------------------------------------

    pub(super) fn restart_informer(&mut self, kind_id: &'static str) {
        if let Some(task) = self.watch_task.take() {
            task.abort();
        }
        let Some(kind) = kinds::by_id(kind_id) else {
            return;
        };
        self.objects.clear();
        self.store_kind = Some(kind_id);
        self.loading = true;
        self.load_started = Instant::now();

        let task = cluster::start_watcher(
            &self.rt,
            self.client.clone(),
            kind,
            self.namespace.clone(),
            cluster::Source::Main,
            forward_watch(self.tx.clone()),
            forward_status(self.tx.clone()),
            self.egui_ctx.clone(),
        );
        self.watch_task = Some(task);

        if kind.id == "pods" {
            self.restart_metrics();
        } else {
            if let Some(task) = self.metrics_task.take() {
                task.abort();
            }
            self.metrics.clear();
        }
    }

    pub(super) fn restart_metrics(&mut self) {
        if let Some(task) = self.metrics_task.take() {
            task.abort();
        }
        self.metrics.clear();
        let task = cluster::start_metrics_watcher(
            &self.rt,
            self.client.clone(),
            self.namespace.clone(),
            forward_watch(self.tx.clone()),
            self.egui_ctx.clone(),
        );
        self.metrics_task = Some(task);
    }

    /// Cluster events for the dashboard; namespace-scoped like everything else.
    pub(super) fn restart_events(&mut self) {
        if let Some(task) = self.events_task.take() {
            task.abort();
        }
        self.events.clear();
        let task = cluster::start_events_poller(
            &self.rt,
            self.client.clone(),
            self.namespace.clone(),
            forward_events(self.tx.clone()),
            self.egui_ctx.clone(),
        );
        self.events_task = Some(task);
    }

    /// Deployments and statefulsets for the dashboard's applications grid;
    /// namespace-scoped like the events feed next to it.
    pub(super) fn restart_apps(&mut self) {
        if let Some(task) = self.apps_task.take() {
            task.abort();
        }
        self.apps.clear();
        let task = cluster::start_apps_poller(
            &self.rt,
            self.client.clone(),
            self.namespace.clone(),
            forward_apps(self.tx.clone()),
            self.egui_ctx.clone(),
        );
        self.apps_task = Some(task);
    }

    /// Cluster-wide pod watch, started on first use: workload logs need to
    /// resolve a selector to the pods it matches, whatever the namespace filter.
    pub(super) fn ensure_pods_store(&mut self) {
        if self.pods_task.is_some() {
            return;
        }
        let task = cluster::start_watcher(
            &self.rt,
            self.client.clone(),
            kinds::by_id("pods").expect("pods in catalog"),
            String::new(),
            cluster::Source::Pods,
            forward_watch(self.tx.clone()),
            forward_status(self.tx.clone()),
            self.egui_ctx.clone(),
        );
        self.pods_task = Some(task);
    }

    /// Events about the object open in the drawer (Vue's pod Overview tab).
    pub(super) fn restart_object_events(&mut self) {
        self.stop_object_events();
        let Some(selection) = &self.selection else {
            return;
        };
        let task = cluster::start_object_events_poller(
            &self.rt,
            self.client.clone(),
            selection.namespace.clone(),
            selection.name.clone(),
            forward_object_events(self.tx.clone()),
            self.egui_ctx.clone(),
        );
        self.object_events_task = Some(task);
    }

    pub(super) fn stop_object_events(&mut self) {
        if let Some(task) = self.object_events_task.take() {
            task.abort();
        }
        self.object_events.clear();
    }

    /// Prometheus usage charts for the open pod (Vue's `PrometheusMetrics`).
    pub(super) fn restart_timeline(&mut self) {
        self.stop_timeline();
        let Some(selection) = self.selection.clone() else {
            return;
        };
        let (namespace, name) = (selection.namespace.clone(), selection.name.clone());

        // Vue charts pods and the workloads whose pods it can resolve
        // (Deployment, StatefulSet, DaemonSet); anything else has no charts.
        let subject = match selection.kind.id {
            "pods" => prometheus::Subject::Pod {
                namespace,
                pod: name,
            },
            "deployments" | "statefulsets" | "daemonsets" => {
                // the selector matches pods that may live in the pods store
                self.ensure_pods_store();
                let labels = selector_labels(&selection.object);
                let pods: Vec<String> =
                    cluster::pods_for_selector(self.pods.values(), &namespace, &labels)
                        .into_iter()
                        .map(|(_, pod)| pod)
                        .collect();
                if pods.is_empty() {
                    self.timeline = Some(Err("no pods for this workload yet".to_string()));
                    return;
                }
                prometheus::Subject::Workload { namespace, pods }
            }
            _ => return,
        };

        let task = prometheus::start(
            &self.rt,
            self.client.clone(),
            subject,
            forward_timeline(self.tx.clone()),
            self.egui_ctx.clone(),
        );
        self.timeline_task = Some(task);
    }

    pub(super) fn stop_timeline(&mut self) {
        if let Some(task) = self.timeline_task.take() {
            task.abort();
        }
        self.timeline = None;
    }

    /// Discover every resource type the cluster serves (fills the picker).
    pub(super) fn start_discovery(&mut self) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            match discovery::all(&client).await {
                Ok(types) => {
                    let _ = tx.send(Msg::ResourceTypes(types));
                }
                Err(message) => cluster::debug_log(&format!("discovery failed: {message}")),
            }
        });
    }
    /// One shell session's news: output to parse, or the end of it.
    pub(super) fn apply_terminal(&mut self, event: TerminalEvent) {
        match event {
            TerminalEvent::Output { id, bytes } => {
                let Some(session) = self.terminals.iter_mut().find(|session| session.id == id)
                else {
                    return;
                };
                // Parsing here, on the UI thread, is what keeps the screen in
                // step with what is drawn from it: there is no second copy of
                // the state to fall behind.
                session.parser.process(&bytes);
                // new output means the view is looking at the past: come back to
                // the live screen, as a terminal does
                session.scroll = 0;
                self.egui_ctx.request_repaint();
            }
            TerminalEvent::Failed { id, error } => {
                if let Some(session) = self.terminals.iter_mut().find(|session| session.id == id) {
                    session.error = Some(error);
                    session.alive = false;
                }
                self.egui_ctx.request_repaint();
            }
            TerminalEvent::Ended { id } => {
                if let Some(session) = self.terminals.iter_mut().find(|session| session.id == id) {
                    session.alive = false;
                }
                self.egui_ctx.request_repaint();
            }
        }
    }

    /// Apply one informer event to the map that owns that source.
    pub(super) fn apply_watch(&mut self, source: cluster::Source, event: cluster::Watch) {
        if matches!(event, cluster::Watch::InitDone) {
            if source == cluster::Source::Main {
                self.loading = false;
            }
            // a store that came back means whatever the watcher was complaining
            // about is no longer true: stop saying it. A failure the app cannot
            // fix for itself (no fix on offer) stays until it is dismissed.
            if self.notice.as_ref().is_some_and(|n| n.fix.is_some()) {
                self.notice = None;
            }
            return;
        }
        let map = match source {
            cluster::Source::Main => &mut self.objects,
            cluster::Source::Pods => &mut self.pods,
            cluster::Source::Metrics => &mut self.metrics,
        };
        match event {
            cluster::Watch::Reset => map.clear(),
            cluster::Watch::Apply(mut obj) => {
                // stored objects never need the apiserver's bookkeeping, and it
                // is a third of a pod's payload: drop it on the way in
                cluster::strip_managed_fields(&mut obj.data);
                map.insert(cluster::obj_key(&obj), *obj);
            }
            cluster::Watch::Delete(key) => {
                map.remove(&key);
            }
            cluster::Watch::InitDone => {}
        }
    }
}
// ---------------------------------------------------------------------------
// channel adapters (cluster tasks write, the UI thread drains)
// ---------------------------------------------------------------------------

/// Best-effort context name: kubeconfig context when there is one, otherwise
/// the in-cluster service account.
pub(super) fn detect_context() -> String {
    if crate::cluster::in_cluster_available() {
        return "in-cluster".to_string();
    }
    kube::config::Kubeconfig::read()
        .ok()
        .and_then(|k| k.current_context)
        .unwrap_or_else(|| "default".to_string())
}

pub(super) fn forward_namespaces(tx: mpsc::Sender<Msg>) -> mpsc::Sender<Vec<String>> {
    let (t, r) = mpsc::channel::<Vec<String>>();
    std::thread::spawn(move || {
        while let Ok(names) = r.recv() {
            let _ = tx.send(Msg::Namespaces(names));
        }
    });
    t
}

/// `spec.selector.matchLabels` of a workload — how its pods are found (the
/// drawer's log aggregation and usage charts both need it).
pub(super) fn selector_labels(object: &DynamicObject) -> HashMap<String, String> {
    object
        .data
        .get("spec")
        .and_then(|spec| spec.get("selector"))
        .and_then(|selector| selector.get("matchLabels"))
        .and_then(|labels| labels.as_object())
        .map(|labels| {
            labels
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn forward_status(tx: mpsc::Sender<Msg>) -> mpsc::Sender<String> {
    let (t, r) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        while let Ok(s) = r.recv() {
            let _ = tx.send(Msg::Status(s));
        }
    });
    t
}

pub(super) fn forward_events(tx: mpsc::Sender<Msg>) -> mpsc::Sender<Vec<cluster::EventRow>> {
    let (t, r) = mpsc::channel::<Vec<cluster::EventRow>>();
    std::thread::spawn(move || {
        while let Ok(rows) = r.recv() {
            if tx.send(Msg::Events(rows)).is_err() {
                break;
            }
        }
    });
    t
}

pub(super) fn forward_apps(tx: mpsc::Sender<Msg>) -> mpsc::Sender<Vec<cluster::AppRow>> {
    let (t, r) = mpsc::channel::<Vec<cluster::AppRow>>();
    std::thread::spawn(move || {
        while let Ok(rows) = r.recv() {
            if tx.send(Msg::Apps(rows)).is_err() {
                break;
            }
        }
    });
    t
}

pub(super) fn forward_object_events(tx: mpsc::Sender<Msg>) -> mpsc::Sender<Vec<cluster::EventRow>> {
    let (t, r) = mpsc::channel::<Vec<cluster::EventRow>>();
    std::thread::spawn(move || {
        while let Ok(rows) = r.recv() {
            if tx.send(Msg::ObjectEvents(rows)).is_err() {
                break;
            }
        }
    });
    t
}

pub(super) fn forward_timeline(
    tx: mpsc::Sender<Msg>,
) -> mpsc::Sender<Result<prometheus::Timeline, String>> {
    let (t, r) = mpsc::channel::<Result<prometheus::Timeline, String>>();
    std::thread::spawn(move || {
        while let Ok(timeline) = r.recv() {
            if tx.send(Msg::Timeline(timeline)).is_err() {
                break;
            }
        }
    });
    t
}

pub(super) fn forward_watch(
    tx: mpsc::Sender<Msg>,
) -> mpsc::Sender<(cluster::Source, cluster::Watch)> {
    let (t, r) = mpsc::channel::<(cluster::Source, cluster::Watch)>();
    std::thread::spawn(move || {
        while let Ok(ev) = r.recv() {
            if tx.send(Msg::Watch(ev.0, ev.1)).is_err() {
                break;
            }
        }
    });
    t
}

pub(super) fn forward_lines(tx: mpsc::Sender<Msg>) -> mpsc::Sender<String> {
    let (t, r) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        while let Ok(line) = r.recv() {
            if notice::is_notice(&line) {
                let _ = tx.send(Msg::Status(line));
            } else {
                let _ = tx.send(Msg::LogLine(line));
            }
        }
    });
    t
}
