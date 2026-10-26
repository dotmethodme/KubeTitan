//! Pod log streaming: following a container's logs and the per-object buffer.

use super::watch::{forward_lines, selector_labels};
use super::*;

impl App {
    pub(crate) fn stop_logs(&mut self) {
        for task in self.log_tasks.drain(..) {
            task.abort();
        }
    }

    /// Containers of the pod open in the drawer (empty for other kinds) — feeds
    /// the log container picker. Init containers come first: they run first, and
    /// their logs are what you want when one of them is stuck.
    pub fn selected_containers(&self) -> Vec<String> {
        self.selection
            .as_ref()
            .map(|sel| cluster::pod_all_containers(&sel.object))
            .unwrap_or_default()
    }

    /// Follow logs for the selected object: straight for a pod, aggregated over
    /// the matching pods for a workload (the Vue drawer's combined logs).
    pub fn start_logs(&mut self) {
        let Some(sel) = self.selection.as_ref() else {
            return;
        };
        let kind = sel.kind;
        let namespace = sel.namespace.clone();
        let name = sel.name.clone();
        let object = sel.object.clone();
        let tx = forward_lines(self.tx.clone());
        let ctx = self.egui_ctx.clone();

        match kind.logs {
            kinds::LogsSupport::Pod => {
                let task = cluster::follow_pod_logs(
                    &self.rt,
                    self.client.clone(),
                    cluster::LogRequest {
                        namespace,
                        pod: name,
                        container: self.log_container.clone(),
                        prefix: None,
                        timestamps: self.log_timestamps,
                    },
                    tx,
                    ctx,
                );
                self.log_tasks.push(task);
            }
            kinds::LogsSupport::Workload => {
                self.ensure_pods_store();
                let labels = selector_labels(&object);
                let pods = cluster::pods_for_selector(self.pods.values(), &namespace, &labels);
                if pods.is_empty() {
                    self.logs
                        .push(notice::status("no pods match this selector yet"));
                    return;
                }
                for (ns, pod) in pods {
                    // workload logs aggregate one pod per stream: tag the stream
                    // with each pod's own first container
                    let container = self
                        .pods
                        .values()
                        .find(|p| {
                            p.metadata.name.as_deref() == Some(pod.as_str())
                                && p.metadata.namespace.as_deref() == Some(ns.as_str())
                        })
                        .and_then(|p| cluster::pod_containers(p).first().cloned());
                    let task = cluster::follow_pod_logs(
                        &self.rt,
                        self.client.clone(),
                        cluster::LogRequest {
                            namespace: ns,
                            pod: pod.clone(),
                            container,
                            prefix: Some(pod),
                            timestamps: self.log_timestamps,
                        },
                        tx.clone(),
                        ctx.clone(),
                    );
                    self.log_tasks.push(task);
                }
            }
            kinds::LogsSupport::None => {}
        }
    }
}
