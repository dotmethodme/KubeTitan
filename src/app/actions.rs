//! Context-menu actions (delete, restart, scale) and the favorites list.

use super::*;

impl App {
    /// Pin a resource type to the sidebar; it becomes usable immediately.
    pub fn add_favorite(&mut self, resource: discovery::ResourceType) {
        kinds::custom_kind(&resource);
        if self
            .favorites
            .iter()
            .any(|saved| saved.id() == resource.id())
        {
            return;
        }
        self.favorites.push(resource);
        self.save_favorites();
    }

    /// Unpin a resource type, leaving any other view open.
    pub fn remove_favorite(&mut self, id: &str) {
        self.favorites.retain(|saved| saved.id() != id);
        self.save_favorites();
        if matches!(self.view, View::Resource(current) if current == id) {
            self.set_view(View::Dashboard);
        }
    }

    /// Move a favorite to a new place in the list and remember the order.
    ///
    /// `to` is the boundary the pointer was over in the list as it stood, so a
    /// move down has to allow for the row itself leaving before the insert.
    pub fn move_favorite(&mut self, from: usize, to: usize) {
        if from >= self.favorites.len() {
            return;
        }
        let mut target = to.min(self.favorites.len());
        let favorite = self.favorites.remove(from);
        if target > from {
            target -= 1;
        }
        target = target.min(self.favorites.len());
        if target == from {
            // put back where it came from: nothing to save
            self.favorites.insert(from, favorite);
            return;
        }
        self.favorites.insert(target, favorite);
        self.save_favorites();
    }

    /// Start editing a secret's or configmap's values: the working copy keeps
    /// the values as the cluster holds them, so the save can send only what the
    /// user actually changed.
    pub fn begin_data_edit(
        &mut self,
        kind: &'static Kind,
        namespace: &str,
        name: &str,
        object: &DynamicObject,
    ) {
        let entries = cluster::data_entries(object, kind.id == "secrets");
        self.data_edit = Some(DataEdit {
            kind,
            namespace: namespace.to_string(),
            name: name.to_string(),
            original: entries.clone(),
            values: entries,
        });
    }

    pub fn cancel_data_edit(&mut self) {
        self.data_edit = None;
    }

    /// Write the values that changed back. Nothing changed means nothing to
    /// send, which is not an error and needs no banner.
    pub fn save_data_edit(&mut self) {
        let Some(edit) = self.data_edit.take() else {
            return;
        };
        let changed = cluster::changed_entries(&edit.original, &edit.values);
        if changed.is_empty() {
            return;
        }
        let patch = cluster::data_patch(&changed, edit.kind.id == "secrets");
        let tx = self.status_tx();
        let task = cluster::patch_data(
            &self.rt,
            self.client.clone(),
            edit.kind,
            edit.namespace,
            edit.name,
            patch,
            tx,
        );
        self.log_tasks.push(task);
    }

    pub(super) fn save_favorites(&mut self) {
        if let Err(message) = favorites::save(&self.favorites) {
            self.flash_error(message);
        }
    }

    // -----------------------------------------------------------------------
    // actions
    // -----------------------------------------------------------------------

    pub fn begin_action(&mut self, kind: &'static Kind, key: &str, action: Action) {
        let Some(obj) = self.objects.get(key) else {
            return;
        };
        if action == Action::Terminal {
            // Not a mutation either: the dock opens and the shell runs there.
            // A pod with several containers gets the first one; a picker would
            // be the place to choose, and that is not what this menu is for.
            let namespace = obj.metadata.namespace.clone().unwrap_or_default();
            let name = obj.metadata.name.clone().unwrap_or_default();
            let container = obj
                .data
                .pointer("/spec/containers/0/name")
                .and_then(|name| name.as_str())
                .unwrap_or_default()
                .to_string();
            crate::terminal::open(self, &namespace, &name, &container);
            return;
        }
        if action == Action::Forward {
            // Not a mutation, so nothing is confirmed: the dialog collects the
            // ports and the forward starts from there. The default remote port
            // comes from the resource, which usually names one.
            let target = if kind.id == "services" {
                portforward::Target::Service
            } else {
                portforward::Target::Pod
            };
            let namespace = obj.metadata.namespace.clone().unwrap_or_default();
            let name = obj.metadata.name.clone().unwrap_or_default();
            let remote_port = default_port(obj, kind.id);
            self.forward_dialog = Some(portforward::Dialog::for_resource(
                &namespace,
                target,
                &name,
                &remote_port,
                &port_hint(obj, kind.id),
            ));
            return;
        }
        let replicas = obj
            .data
            .get("spec")
            .and_then(|s| s.get("replicas"))
            .and_then(|r| r.as_i64())
            .unwrap_or(0);
        self.pending = Some(Pending {
            kind,
            namespace: obj.metadata.namespace.clone().unwrap_or_default(),
            name: obj.metadata.name.clone().unwrap_or_default(),
            action,
            replicas,
            current_replicas: replicas,
        });
    }

    pub fn set_scale_target(&mut self, value: i64) {
        if let Some(pending) = self.pending.as_mut() {
            pending.replicas = value.max(0);
        }
    }

    pub fn scale_target(&self) -> i64 {
        self.pending.as_ref().map_or(0, |p| p.replicas)
    }

    /// Cancel the pending action (backdrop click, Cancel button, Esc).
    pub fn cancel_pending(&mut self) {
        self.pending = None;
    }

    /// Execute the confirmed action.
    pub fn confirm_pending(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let tx = self.status_tx();
        match pending.action {
            Action::Delete => {
                let task = cluster::delete(
                    &self.rt,
                    self.client.clone(),
                    pending.kind,
                    pending.namespace.clone(),
                    pending.name.clone(),
                    tx,
                );
                self.log_tasks.push(task);
            }
            Action::Restart => {
                let task = cluster::restart(
                    &self.rt,
                    self.client.clone(),
                    pending.kind,
                    pending.namespace.clone(),
                    pending.name.clone(),
                    tx,
                );
                self.log_tasks.push(task);
            }
            Action::Scale => {
                let task = cluster::scale(
                    &self.rt,
                    self.client.clone(),
                    pending.kind,
                    pending.namespace.clone(),
                    pending.name.clone(),
                    pending.replicas,
                    tx,
                );
                self.log_tasks.push(task);
            }
            Action::Forward => {
                // unreachable: `begin_action` opens the forward dialog instead
                // of queueing a confirmation
            }
            Action::Terminal => {
                // unreachable: `begin_action` opens the dock instead of queueing
                // a confirmation
            }
        }
    }
}

/// The port a forward should aim at by default: the service's first port, or
/// the pod's first declared container port.
fn default_port(object: &kube::core::DynamicObject, kind_id: &str) -> String {
    let pointer = if kind_id == "services" {
        "/spec/ports/0/port"
    } else {
        "/spec/containers/0/ports/0/containerPort"
    };
    object
        .data
        .pointer(pointer)
        .and_then(|port| port.as_i64())
        .map(|port| port.to_string())
        .unwrap_or_default()
}

/// What the target listens on, as the dialog says it under the port fields.
///
/// A service's `port` and `targetPort` are different numbers on purpose — 80 is
/// what a client connects to, 8080 is what the pod happens to serve on — and the
/// remote port to forward to is the service's own. Spelling both out is what
/// stops the target port from being typed in by mistake.
fn port_hint(object: &kube::core::DynamicObject, kind_id: &str) -> String {
    let list = match object
        .data
        .pointer("/spec/ports")
        .and_then(|ports| ports.as_array())
    {
        Some(list) => list,
        None => match object
            .data
            .pointer("/spec/containers/0/ports")
            .and_then(|ports| ports.as_array())
        {
            Some(list) => list,
            None => return String::new(),
        },
    };
    let service = kind_id == "services";
    list.iter()
        .filter_map(|entry| {
            let port = entry
                .get("port")
                .or_else(|| entry.get("containerPort"))?
                .as_i64()?;
            let target = entry
                .get("targetPort")
                .map(|target| match target {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => port.to_string(),
                })
                .unwrap_or_else(|| port.to_string());
            Some(if service && target != port.to_string() {
                format!("{port} (\u{2192} {target})")
            } else {
                port.to_string()
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
