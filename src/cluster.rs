//! Cluster layer.
//!
//! Every resource type is handled as a [`DynamicObject`]: the catalog supplies
//! an [`ApiResource`], so one watcher implementation serves pods, deployments
//! and CRDs alike.
//!
//! kube 4.x note: `reflector::Store` cannot hold `DynamicObject` (its
//! `DynamicType` is `ApiResource`, which has no `Default`), so this module
//! consumes the raw `watcher` event stream and the UI thread keeps the objects
//! in plain maps. That also avoids cloning `Arc` handles every frame.
//!
//! Log streaming is the one typed path: `Api::log_stream` is implemented only
//! for `k8s_openapi::api::core::v1::Pod`.

use eframe::egui;
use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    Client,
    api::{Api, DeleteParams, ListParams, LogParams, Patch, PatchParams},
    config::KubeConfigOptions,
    core::{ApiResource, DynamicObject, GroupVersionKind},
    runtime::watcher::{Config as WatcherConfig, Event, watcher},
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Duration;
use tokio::task::JoinHandle;

use crate::kinds::{self, Kind};
use crate::notice;

/// Lines of history requested when a pod log stream opens (`kubectl logs --tail`).
pub const LOG_TAIL_LINES: i64 = 1000;
/// How many times a dropped log follow is retried before reporting an error.
pub const LOG_RECONNECTS: u32 = 5;
/// Hard cap on retained log lines per drawer.
pub const LOG_BUFFER_LINES: usize = 5000;
/// `metrics.k8s.io` cannot be watched, so it is polled.
pub const METRICS_POLL_INTERVAL: Duration = Duration::from_secs(20);
/// Dashboard event feed refresh.
pub const EVENTS_POLL_INTERVAL: Duration = Duration::from_secs(30);
/// How many events the dashboard keeps in view.
pub const EVENTS_SHOWN: usize = 12;
/// The drawer's per-object event list refreshes faster: it is a detail view.
pub const OBJECT_EVENTS_POLL_INTERVAL: Duration = Duration::from_secs(10);
/// The dashboard's applications grid re-lists deployments and statefulsets.
pub const APPS_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Server-side apply records the writer in `metadata.managedFields`; the Wails
/// app writes under this same name, so the two clients do not fight each other.
pub const FIELD_MANAGER: &str = "kubetitan";

/// Which informer an event came from — the UI keeps one map per source.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// the resource list currently on screen
    Main,
    /// cluster-wide pods (workload log aggregation, dashboard counts)
    Pods,
    /// `metrics.k8s.io` PodMetrics
    Metrics,
}

/// A state mutation, ready to be applied to the UI-side map.
pub enum Watch {
    /// A fresh initial list is starting: drop what we had.
    Reset,
    /// Add or update one object.
    Apply(Box<DynamicObject>),
    /// Remove by `namespace/name`.
    Delete(String),
    /// The initial list is complete: what we hold is now authoritative.
    InitDone,
}

/// `namespace/name` — the identity used everywhere in the UI.
pub fn obj_key(obj: &DynamicObject) -> String {
    format!(
        "{}/{}",
        obj.metadata.namespace.clone().unwrap_or_default(),
        obj.metadata.name.clone().unwrap_or_default(),
    )
}

pub fn api_for(client: Client, kind: &Kind, ns: &str) -> Api<DynamicObject> {
    let ar = kind.api_resource();
    if kind.namespaced && !ns.is_empty() {
        Api::namespaced_with(client, ns, &ar)
    } else {
        Api::all_with(client, &ar)
    }
}

/// Keep the log buffer bounded; called by the UI after each append.
pub fn cap_logs(logs: &mut Vec<String>) {
    if logs.len() > LOG_BUFFER_LINES {
        let excess = logs.len() - LOG_BUFFER_LINES;
        logs.drain(..excess);
    }
}

/// Watch one resource type, forwarding changes to the UI thread (which owns the
/// map) and requesting a repaint on every event so the table follows along.
#[allow(clippy::too_many_arguments)]
pub fn start_watcher(
    rt: &tokio::runtime::Runtime,
    client: Client,
    kind: &'static Kind,
    ns: String,
    source: Source,
    tx: Sender<(Source, Watch)>,
    status: Sender<String>,
    egui_ctx: egui::Context,
) -> JoinHandle<()> {
    let api = api_for(client, kind, &ns);
    let label = kind.label.to_string();
    rt.spawn(async move {
        let mut stream = Box::pin(watcher(api, WatcherConfig::default()));
        while let Some(event) = stream.next().await {
            egui_ctx.request_repaint();
            match event {
                Ok(Event::Init) => send(&tx, source, Watch::Reset),
                Ok(Event::InitApply(obj)) | Ok(Event::Apply(obj)) => {
                    send(&tx, source, Watch::Apply(Box::new(obj)))
                }
                Ok(Event::Delete(obj)) => send(&tx, source, Watch::Delete(obj_key(&obj))),
                Ok(Event::InitDone) => send(&tx, source, Watch::InitDone),
                Err(e) => {
                    // the watcher retries internally; stop the skeleton so the
                    // view does not spin forever, then say why we stalled — in
                    // that order, so the failure is the last word and the banner
                    // survives the view going quiet
                    if source == Source::Main {
                        send(&tx, source, Watch::InitDone);
                    }
                    let _ = status.send(notice::fail(format!("{label} watcher: {e}")));
                }
            }
        }
    })
}

/// Watch `metrics.k8s.io/v1beta1` PodMetrics for the pods view CPU/Memory
/// columns. A missing metrics-server is not an error.
pub fn start_metrics_watcher(
    rt: &tokio::runtime::Runtime,
    client: Client,
    ns: String,
    tx: Sender<(Source, Watch)>,
    egui_ctx: egui::Context,
) -> JoinHandle<()> {
    let ar = ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics"),
        "pods",
    );
    let api: Api<DynamicObject> = if ns.is_empty() {
        Api::all_with(client, &ar)
    } else {
        Api::namespaced_with(client, &ns, &ar)
    };
    rt.spawn(async move {
        // metrics.k8s.io rejects `watch` ("no metadata.resourceVersion in watch
        // result"), so this is a poll loop, not a watcher. Empty namespace =
        // cluster-wide, exactly what `kubectl top pods -A` does.
        let mut ticker = tokio::time::interval(METRICS_POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_error = String::new();
        loop {
            ticker.tick().await;
            match api.list(&ListParams::default()).await {
                Ok(list) => {
                    last_error.clear();
                    send(&tx, Source::Metrics, Watch::Reset);
                    for item in list.items {
                        send(&tx, Source::Metrics, Watch::Apply(Box::new(item)));
                    }
                    send(&tx, Source::Metrics, Watch::InitDone);
                    egui_ctx.request_repaint();
                }
                Err(e) => {
                    // metrics-server is optional: stay quiet unless the reason
                    // changes, and stop showing stale numbers.
                    let msg = e.to_string();
                    if msg != last_error {
                        debug_log(&format!("metrics list failed: {msg}"));
                        last_error = msg;
                        send(&tx, Source::Metrics, Watch::Reset);
                        egui_ctx.request_repaint();
                    }
                }
            }
        }
    })
}

fn send(tx: &Sender<(Source, Watch)>, source: Source, event: Watch) {
    let _ = tx.send((source, event));
}

/// Opt-in diagnostics (`KT_DEBUG=1`): appends to `/tmp/kt/app-debug.log`.
pub fn debug_log(line: &str) {
    if std::env::var("KT_DEBUG").is_ok() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/kt/app-debug.log")
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Rebuild the pod metrics lookup from the metrics objects.
pub fn metrics_index<'a>(pods: impl Iterator<Item = &'a DynamicObject>) -> kinds::MetricsIndex {
    let mut idx = kinds::MetricsIndex::default();
    for pod in pods {
        let ns = pod.metadata.namespace.clone().unwrap_or_default();
        let name = pod.metadata.name.clone().unwrap_or_default();
        let Some(containers) = pod.data.get("containers").and_then(|c| c.as_array()) else {
            continue;
        };
        let mut cpu = 0u64;
        let mut mem = 0u64;
        for c in containers {
            let usage = c.get("usage");
            cpu += usage
                .and_then(|u| u.get("cpu"))
                .and_then(|v| v.as_str())
                .map(parse_cpu_milli)
                .unwrap_or(0);
            mem += usage
                .and_then(|u| u.get("memory"))
                .and_then(|v| v.as_str())
                .map(parse_mem_bytes)
                .unwrap_or(0);
        }
        idx.insert(&ns, &name, cpu, mem);
    }
    idx
}

/// `100m`, `1`, `250000000n` → millicores.
fn parse_cpu_milli(q: &str) -> u64 {
    if let Some(n) = q.strip_suffix('n') {
        n.parse::<u64>().unwrap_or(0) / 1_000_000
    } else if let Some(m) = q.strip_suffix('m') {
        m.parse::<u64>().unwrap_or(0)
    } else if let Some(u) = q.strip_suffix('u') {
        u.parse::<u64>().unwrap_or(0) / 1_000
    } else {
        q.parse::<f64>().unwrap_or(0.0) as u64 * 1000
    }
}

/// `30Mi`, `1234567`, `2Gi` → bytes.
fn parse_mem_bytes(q: &str) -> u64 {
    const KIB: f64 = 1024.0;
    let (num, mult) = if let Some(v) = q.strip_suffix("Ki") {
        (v, KIB)
    } else if let Some(v) = q.strip_suffix("Mi") {
        (v, KIB * KIB)
    } else if let Some(v) = q.strip_suffix("Gi") {
        (v, KIB * KIB * KIB)
    } else if let Some(v) = q.strip_suffix('k') {
        (v, 1000.0)
    } else if let Some(v) = q.strip_suffix('M') {
        (v, 1e6)
    } else if let Some(v) = q.strip_suffix('G') {
        (v, 1e9)
    } else {
        (q, 1.0)
    };
    (num.parse::<f64>().unwrap_or(0.0) * mult) as u64
}

/// Namespace names for the header selector.
pub fn list_namespaces(
    rt: &tokio::runtime::Runtime,
    client: Client,
    tx: Sender<Vec<String>>,
) -> JoinHandle<()> {
    rt.spawn(async move {
        let ar = ApiResource::from_gvk_with_plural(
            &GroupVersionKind::gvk("", "v1", "Namespace"),
            "namespaces",
        );
        match Api::<DynamicObject>::all_with(client, &ar)
            .list(&ListParams::default())
            .await
        {
            Ok(list) => {
                let mut names: Vec<String> = list
                    .items
                    .into_iter()
                    .filter_map(|ns| ns.metadata.name)
                    .collect();
                names.sort();
                let _ = tx.send(names);
            }
            Err(e) => {
                let _ = tx.send(Vec::new());
                log::warn!("namespaces: {e:#}");
            }
        }
    })
}

/// Container names out of one of a pod's container lists, in spec order.
fn spec_container_names(pod: &kube::core::DynamicObject, field: &str) -> Vec<String> {
    pod.data
        .get("spec")
        .and_then(|spec| spec.get(field))
        .and_then(|containers| containers.as_array())
        .map(|containers| {
            containers
                .iter()
                .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// App container names of a pod, in spec order — the drawer's default log target.
pub fn pod_containers(pod: &kube::core::DynamicObject) -> Vec<String> {
    spec_container_names(pod, "containers")
}

/// Init container names, in spec order. They run before the app containers, and
/// a crash-looping init container is exactly the one whose logs you need, so
/// they are addressable as log targets too.
pub fn pod_init_containers(pod: &kube::core::DynamicObject) -> Vec<String> {
    spec_container_names(pod, "initContainers")
}

/// Ephemeral (debug) container names — `kubectl debug` copies.
pub fn pod_ephemeral_containers(pod: &kube::core::DynamicObject) -> Vec<String> {
    spec_container_names(pod, "ephemeralContainers")
}

/// Every container in the pod, init first: the order the drawer and the log
/// picker show, so an init container is clickable before the app containers.
pub fn pod_all_containers(pod: &kube::core::DynamicObject) -> Vec<String> {
    let mut names = pod_init_containers(pod);
    names.extend(pod_containers(pod));
    names.extend(pod_ephemeral_containers(pod));
    names
}

/// One pod's log follow: which pod, which container, and how to tag its lines.
pub struct LogRequest {
    pub namespace: String,
    pub pod: String,
    /// a multi-container pod rejects a log request without a container
    pub container: Option<String>,
    /// set when aggregating workload logs, so lines carry their source pod
    pub prefix: Option<String>,
    /// ask the apiserver to stamp each line. Timestamps are not something the
    /// pane can add while drawing: the stream has to request them, so switching
    /// this on means a new request.
    pub timestamps: bool,
}

/// Follow one pod's logs, reconnecting when the apiserver drops the stream.
/// `request.prefix` is set when aggregating workload logs, so lines are tagged
/// with their source pod, like the Vue drawer's `show-source`.
pub fn follow_pod_logs(
    rt: &tokio::runtime::Runtime,
    client: Client,
    request: LogRequest,
    tx: Sender<String>,
    egui_ctx: egui::Context,
) -> JoinHandle<()> {
    let LogRequest {
        namespace: ns,
        pod: name,
        container,
        prefix,
        timestamps,
    } = request;
    rt.spawn(async move {
        let api: Api<Pod> = Api::namespaced(client, &ns);
        let mut lp = LogParams {
            follow: true,
            tail_lines: Some(LOG_TAIL_LINES),
            container,
            timestamps,
            ..Default::default()
        };
        let decorate = |line: String| match &prefix {
            Some(p) => format!("{p} | {line}"),
            None => line,
        };

        // A pending pod has no logs yet and the apiserver answers 400
        // "container x in pod y is waiting to start"; kubectl -f waits, so do we.
        // A followed stream also drops on proxy timeouts and apiserver
        // restarts — kubectl reconnects, so we reconnect, and only report an
        // error once retrying stops helping.
        let mut lines_seen = false;
        let mut drops = 0_u32;
        loop {
            match api.log_stream(&name, &lp).await {
                Ok(stream) => {
                    let mut lines = Box::pin(stream).lines();
                    loop {
                        match lines.next().await {
                            Some(Ok(line)) => {
                                egui_ctx.request_repaint();
                                lines_seen = true;
                                let _ = tx.send(decorate(line));
                            }
                            Some(Err(e)) => {
                                debug_log(&format!("log stream dropped: {e}"));
                                break;
                            }
                            None => {
                                let _ = tx.send(notice::status("stream closed"));
                                return;
                            }
                        }
                    }

                    drops += 1;
                    if drops > LOG_RECONNECTS {
                        let _ = tx.send(notice::err(format!(
                            "log stream dropped {drops} times, giving up"
                        )));
                        return;
                    }
                    if lines_seen {
                        let _ = tx.send(notice::status("log stream interrupted, reconnecting…"));
                        // reconnecting prints only what is written from now on
                        lp.tail_lines = Some(0);
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("waiting to start") {
                        let _ = tx.send(notice::status("container waiting to start…"));
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                    let _ = tx.send(notice::err(format!("open stream: {msg}")));
                    return;
                }
            }
        }
    })
}

/// Delete an object, reporting the outcome on the status channel.
pub fn delete(
    rt: &tokio::runtime::Runtime,
    client: Client,
    kind: &'static Kind,
    ns: String,
    name: String,
    tx: Sender<String>,
) -> JoinHandle<()> {
    rt.spawn(async move {
        let api = api_for(client, kind, &ns);
        let msg = match api.delete(&name, &DeleteParams::default()).await {
            Ok(_) => notice::status(format!("deleted {} {name}", kind.lower())),
            Err(e) => notice::err(format!("delete {name}: {e}")),
        };
        let _ = tx.send(msg);
    })
}

/// Rolling restart: bump the pod template's `restartedAt` annotation, exactly
/// what `kubectl rollout restart` does.
pub fn restart(
    rt: &tokio::runtime::Runtime,
    client: Client,
    kind: &'static Kind,
    ns: String,
    name: String,
    tx: Sender<String>,
) -> JoinHandle<()> {
    rt.spawn(async move {
        let api = api_for(client, kind, &ns);
        let now = jiff::Timestamp::now().to_string();
        let patch = json!({
            "spec": { "template": { "metadata": { "annotations": {
                "kubectl.kubernetes.io/restartedAt": now
            }}}}
        });
        let msg = match api
            .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
        {
            Ok(_) => notice::status(format!("restarted {} {name}", kind.lower())),
            Err(e) => notice::err(format!("restart {name}: {e}")),
        };
        let _ = tx.send(msg);
    })
}

/// Scale a workload.
pub fn scale(
    rt: &tokio::runtime::Runtime,
    client: Client,
    kind: &'static Kind,
    ns: String,
    name: String,
    replicas: i64,
    tx: Sender<String>,
) -> JoinHandle<()> {
    rt.spawn(async move {
        let api = api_for(client, kind, &ns);
        let patch = json!({ "spec": { "replicas": replicas } });
        let msg = match api
            .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
        {
            Ok(_) => notice::status(format!("scaled {name} to {replicas}")),
            Err(e) => notice::err(format!("scale {name}: {e}")),
        };
        let _ = tx.send(msg);
    })
}

/// Drops `metadata.managedFields` from an object's data. It is the apiserver's
/// own bookkeeping — 36% of a pod's payload in this cluster — and the app never
/// shows it (the Wails backend's `GetResourceYaml` strips it too).
pub fn strip_managed_fields(value: &mut serde_json::Value) {
    if let Some(metadata) = value.get_mut("metadata").and_then(|m| m.as_object_mut()) {
        metadata.remove("managedFields");
    }
}

/// A secret's `data` is base64 and a configmap's is already text; decoding is
/// what makes the drawer readable, so it happens in one place. A value that is
/// not valid base64 — a hand-edited secret can hold anything — is shown as it
/// stands rather than dropped, and the list is sorted by key so the tab reads
/// the same way `kubectl get secret -o jsonpath` does.
pub fn data_entries(object: &DynamicObject, is_secret: bool) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = object
        .data
        .get("data")
        .and_then(|data| data.as_object())
        .map(|map| {
            map.iter()
                .map(|(key, value)| {
                    let raw = value.as_str().unwrap_or_default();
                    let text = match (is_secret, decode_base64(raw)) {
                        (true, Some(text)) => text,
                        _ => raw.to_string(),
                    };
                    (key.clone(), text)
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn decode_base64(text: &str) -> Option<String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Which entries the user actually changed. A key left alone is not in the
/// patch, so it is not rewritten.
pub fn changed_entries(
    original: &[(String, String)],
    edited: &[(String, String)],
) -> Vec<(String, String)> {
    edited
        .iter()
        .filter(|(key, value)| {
            original
                .iter()
                .any(|(was_key, was_value)| was_key == key && was_value != value)
        })
        .cloned()
        .collect()
}

/// The merge patch for an edit: the changed keys only, base64-encoded again when
/// the object is a secret.
pub fn data_patch(changed: &[(String, String)], is_secret: bool) -> serde_json::Value {
    use base64::Engine as _;
    let mut data = serde_json::Map::new();
    for (key, value) in changed {
        let encoded = if is_secret {
            base64::engine::general_purpose::STANDARD.encode(value)
        } else {
            value.clone()
        };
        data.insert(key.clone(), serde_json::Value::String(encoded));
    }
    serde_json::json!({ "data": data })
}

/// Write a secret's or configmap's values back.
pub fn patch_data(
    rt: &tokio::runtime::Runtime,
    client: Client,
    kind: &'static Kind,
    ns: String,
    name: String,
    data: serde_json::Value,
    tx: Sender<String>,
) -> JoinHandle<()> {
    rt.spawn(async move {
        let api = api_for(client, kind, &ns);
        let patch = serde_json::json!({ "data": data });
        let msg = match api
            .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
        {
            Ok(_) => notice::status(format!("saved {} {name}", kind.lower())),
            Err(e) => notice::err(format!("save {name}: {e}")),
        };
        let _ = tx.send(msg);
    })
}

/// Server-side apply, the equivalent of the Wails app's
/// `kubectl apply --server-side --field-manager kubetitan [--force-conflicts]`:
/// the object's identity is read from the YAML itself, as kubectl reads it from
/// the file. Returns kubectl's sentence (`deployment.apps/blog configured`).
pub fn apply_yaml(
    rt: &tokio::runtime::Runtime,
    client: Client,
    kind: &'static Kind,
    yaml: String,
    force: bool,
    tx: Sender<Result<String, String>>,
) -> JoinHandle<()> {
    rt.spawn(async move {
        let object: DynamicObject = match serde_yaml::from_str(&yaml) {
            Ok(object) => object,
            Err(e) => {
                let _ = tx.send(Err(format!("invalid YAML: {e}")));
                return;
            }
        };
        let Some(name) = object.metadata.name.clone() else {
            let _ = tx.send(Err("the YAML has no metadata.name".to_string()));
            return;
        };
        let ns = object
            .metadata
            .namespace
            .clone()
            .unwrap_or_else(|| "default".to_string());
        // the resourceVersion we sent back tells us whether the apiserver
        // changed anything: SSA does not bump it on a no-op, like kubectl's
        // "unchanged"
        let sent_rv = object.metadata.resource_version.clone();

        let api = api_for(client, kind, &ns);
        let params = if force {
            // `PatchParams::force` flips force_conflicts on; kubectl spells the
            // same thing `--force-conflicts`
            PatchParams::apply(FIELD_MANAGER).force()
        } else {
            PatchParams::apply(FIELD_MANAGER)
        };
        let result = api.patch(&name, &params, &Patch::Apply(&object)).await;

        let _ = tx.send(match result {
            Ok(applied) => {
                let qualified = if kind.group.is_empty() {
                    kind.plural.to_string()
                } else {
                    format!("{}.{}", kind.plural, kind.group)
                };
                let verb = match (&sent_rv, &applied.metadata.resource_version) {
                    (Some(sent), Some(now)) if sent == now => "unchanged",
                    _ => "configured",
                };
                Ok(format!("{qualified}/{name} {verb}"))
            }
            // the apiserver's own wording here is longer than it is useful
            Err(kube::Error::Api(err))
                if err.message.contains("may not change fields other than") =>
            {
                Err(
                    "cannot apply Pod spec changes: most Pod fields are immutable after creation. \
                     Edit the owning controller (for example StatefulSet/Deployment) instead, or \
                     only change Pod metadata like labels/annotations"
                        .to_string(),
                )
            }
            // the apiserver's status message is what kubectl prints; the rest of
            // the error value is Rust debug noise in a UI
            Err(kube::Error::Api(err)) => Err(err.message),
            Err(e) => Err(format!("{e}")),
        });
    })
}

/// Fetch one object as YAML for the drawer's YAML tab.
pub fn fetch_yaml(
    rt: &tokio::runtime::Runtime,
    client: Client,
    kind: &'static Kind,
    ns: String,
    name: String,
    tx: Sender<Result<String, String>>,
) -> JoinHandle<()> {
    rt.spawn(async move {
        let api = api_for(client, kind, &ns);
        let value = match api.get(&name).await {
            Ok(obj) => serde_json::to_value(obj).map(|mut value| {
                strip_managed_fields(&mut value);
                value
            }),
            Err(e) => {
                let _ = tx.send(Err(format!("{e}")));
                return;
            }
        };
        match value {
            Ok(v) => {
                let y = serde_yaml::to_string(&v).unwrap_or_else(|e| format!("# yaml error: {e}"));
                let _ = tx.send(Ok(y));
            }
            Err(e) => {
                let _ = tx.send(Err(format!("{e}")));
            }
        }
    })
}

/// One row of the dashboard's events feed.
#[derive(Clone, Debug)]
pub struct EventRow {
    pub object_kind: String,
    pub object_name: String,
    pub namespace: String,
    pub reason: String,
    pub message: String,
    pub warning: bool,
    /// absolute timestamp (unix seconds) of the event's last update
    pub at: i64,
}

/// Recent cluster events for the dashboard. Events are not a browsable list
/// view, so this reports finished rows rather than raw objects.
pub fn start_events_poller(
    rt: &tokio::runtime::Runtime,
    client: Client,
    ns: String,
    tx: Sender<Vec<EventRow>>,
    egui_ctx: egui::Context,
) -> JoinHandle<()> {
    let ar = ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk("", "v1", "Event"), "events");
    rt.spawn(async move {
        let api: Api<DynamicObject> = if ns.is_empty() {
            Api::all_with(client, &ar)
        } else {
            Api::namespaced_with(client, &ns, &ar)
        };
        let mut ticker = tokio::time::interval(EVENTS_POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_error = String::new();
        loop {
            ticker.tick().await;
            match api.list(&ListParams::default()).await {
                Ok(list) => {
                    last_error.clear();
                    let now = jiff::Timestamp::now().as_second();
                    let mut rows: Vec<EventRow> =
                        list.items.into_iter().map(|e| event_row(&e, now)).collect();
                    // newest first; the feed is a "what just happened" list
                    rows.sort_by_key(|e| std::cmp::Reverse(e.at));
                    rows.truncate(EVENTS_SHOWN);
                    let _ = tx.send(rows);
                    egui_ctx.request_repaint();
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg != last_error {
                        debug_log(&format!("events list failed: {msg}"));
                        last_error = msg;
                    }
                }
            }
        }
    })
}

/// Context names the user can switch between: the kubeconfig's contexts, with
/// the in-cluster service account first when that is what we are running as.
pub fn context_names() -> Vec<String> {
    let mut names: Vec<String> = kube::config::Kubeconfig::read()
        .map(|kubeconfig| {
            kubeconfig
                .contexts
                .iter()
                .map(|named| named.name.clone())
                .collect()
        })
        .unwrap_or_default();
    if in_cluster_available() {
        names.insert(0, "in-cluster".to_string());
    }
    if names.is_empty() {
        names.push("default".to_string());
    }
    names
}

/// Does kube resolve to the pod's own account here? It does when the in-cluster
/// environment is there and no kubeconfig is named — the order kube's own
/// resolution follows. Asking this in one place is what keeps the context list
/// honest: every name in it is a name `connect` can be given.
pub fn in_cluster_available() -> bool {
    std::env::var_os("KUBERNETES_SERVICE_HOST").is_some()
        && std::env::var_os("KUBECONFIG").is_none()
}

/// Connect to a named context, or to the pod's own account when there is no
/// name. "in-cluster" means in-cluster: kube's `try_default` would rather use a
/// kubeconfig when one is named, and then the context the user picked would not
/// be the cluster they are talking to. `connect_default` is that other
/// resolution, for the one caller that wants it.
pub async fn connect(context: Option<&str>) -> Result<Client, String> {
    match context {
        None | Some("in-cluster") => {
            let config = kube::Config::incluster().map_err(|e| format!("in-cluster: {e}"))?;
            Client::try_from(config).map_err(|e| e.to_string())
        }
        Some(name) => {
            let kubeconfig = kube::config::Kubeconfig::read().map_err(|e| e.to_string())?;
            let options = KubeConfigOptions {
                context: Some(name.to_string()),
                ..Default::default()
            };
            let config = kube::Config::from_custom_kubeconfig(kubeconfig, &options)
                .await
                .map_err(|e| e.to_string())?;
            Client::try_from(config).map_err(|e| e.to_string())
        }
    }
}

/// kube's own resolution order — the in-cluster account if there is one, the
/// kubeconfig otherwise. For the one case that wants it: the context the app
/// reopened on has gone away, so nothing was chosen and anything will do.
pub async fn connect_default() -> Result<Client, String> {
    Client::try_default().await.map_err(|e| e.to_string())
}

/// One card in the dashboard's applications grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppRow {
    pub name: String,
    /// `deployment` or `statefulset`, as the Vue card labels them.
    pub kind: &'static str,
    pub ready: i32,
    pub desired: i32,
}

/// The Vue grid's three health states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppStatus {
    Healthy,
    Degraded,
    Unavailable,
}

impl AppRow {
    /// Everything ready, some ready, none ready — the Vue grid's rule exactly,
    /// including "scaled to zero is not healthy".
    pub fn status(&self) -> AppStatus {
        if self.ready == self.desired && self.desired > 0 {
            AppStatus::Healthy
        } else if self.ready > 0 {
            AppStatus::Degraded
        } else {
            AppStatus::Unavailable
        }
    }

    /// The catalog id of the list view a card click opens.
    pub fn view_id(&self) -> &'static str {
        match self.kind {
            "statefulset" => "statefulsets",
            _ => "deployments",
        }
    }
}

/// Deployments and statefulsets for the dashboard grid. The Vue dashboard
/// re-lists these every 5 s; a 30 s tick is enough for a card view and keeps
/// the cadence of the events feed.
pub fn start_apps_poller(
    rt: &tokio::runtime::Runtime,
    client: Client,
    ns: String,
    tx: Sender<Vec<AppRow>>,
    egui_ctx: egui::Context,
) -> JoinHandle<()> {
    let sources: [(&'static str, ApiResource); 2] = [
        (
            "deployment",
            ApiResource::from_gvk_with_plural(
                &GroupVersionKind::gvk("apps", "v1", "Deployment"),
                "deployments",
            ),
        ),
        (
            "statefulset",
            ApiResource::from_gvk_with_plural(
                &GroupVersionKind::gvk("apps", "v1", "StatefulSet"),
                "statefulsets",
            ),
        ),
    ];
    rt.spawn(async move {
        let mut ticker = tokio::time::interval(APPS_POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_error = String::new();
        loop {
            ticker.tick().await;
            let mut rows = Vec::new();
            let mut failure = None;
            for (kind, ar) in &sources {
                let api: Api<DynamicObject> = if ns.is_empty() {
                    Api::all_with(client.clone(), ar)
                } else {
                    Api::namespaced_with(client.clone(), &ns, ar)
                };
                match api.list(&ListParams::default()).await {
                    Ok(list) => rows.extend(list.items.iter().map(|o| app_row(kind, o))),
                    Err(e) => failure = Some(e.to_string()),
                }
            }
            if let Some(msg) = failure {
                // a half-listed grid would make cards appear and vanish; wait
                // for a clean pass instead
                if msg != last_error {
                    debug_log(&format!("apps list failed: {msg}"));
                    last_error = msg;
                }
                continue;
            }
            last_error.clear();
            rows.sort_by(|a, b| a.name.cmp(&b.name));
            let _ = tx.send(rows);
            egui_ctx.request_repaint();
        }
    })
}

fn app_row(kind: &'static str, obj: &DynamicObject) -> AppRow {
    let int_at = |path: &[&str]| -> i32 {
        let mut cur = &obj.data;
        for part in path {
            cur = match cur.get(*part) {
                Some(v) => v,
                None => return 0,
            };
        }
        cur.as_i64().unwrap_or(0) as i32
    };
    AppRow {
        name: obj.metadata.name.clone().unwrap_or_default(),
        kind,
        ready: int_at(&["status", "readyReplicas"]),
        desired: int_at(&["spec", "replicas"]),
    }
}

fn event_row(event: &DynamicObject, now: i64) -> EventRow {
    let data = &event.data;
    let get = |path: &[&str]| -> String {
        let mut cur = data;
        for part in path {
            cur = match cur.get(*part) {
                Some(v) => v,
                None => return String::new(),
            };
        }
        cur.as_str().unwrap_or_default().to_string()
    };
    let stamp = get(&["lastTimestamp"]);
    let stamp = if stamp.is_empty() {
        get(&["eventTime"])
    } else {
        stamp
    };
    let at = crate::kinds::parse_secs(&stamp).unwrap_or(now);
    EventRow {
        object_kind: get(&["involvedObject", "kind"]),
        object_name: get(&["involvedObject", "name"]),
        namespace: event.metadata.namespace.clone().unwrap_or_default(),
        reason: get(&["reason"]),
        message: get(&["message"]),
        warning: get(&["type"]) == "Warning",
        at,
    }
}

/// Pods a workload owns, resolved through its label selector — used to
/// aggregate workload logs in the drawer.
pub fn pods_for_selector<'a>(
    pods: impl Iterator<Item = &'a DynamicObject>,
    ns: &str,
    match_labels: &HashMap<String, String>,
) -> Vec<(String, String)> {
    if match_labels.is_empty() {
        return Vec::new();
    }
    pods.filter(|pod| {
        pod.metadata.namespace.as_deref() == Some(ns)
            && pod
                .metadata
                .labels
                .as_ref()
                .is_some_and(|l| match_labels.iter().all(|(k, v)| l.get(k) == Some(v)))
    })
    .map(|pod| {
        (
            pod.metadata.namespace.clone().unwrap_or_default(),
            pod.metadata.name.clone().unwrap_or_default(),
        )
    })
    .collect()
}

/// Events about a single object, for the drawer's Events section. Events carry
/// no usable watch identity, so this polls like the dashboard feed.
pub fn start_object_events_poller(
    rt: &tokio::runtime::Runtime,
    client: Client,
    ns: String,
    name: String,
    tx: Sender<Vec<EventRow>>,
    egui_ctx: egui::Context,
) -> JoinHandle<()> {
    let ar = ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk("", "v1", "Event"), "events");
    rt.spawn(async move {
        let api: Api<DynamicObject> = if ns.is_empty() {
            Api::all_with(client, &ar)
        } else {
            Api::namespaced_with(client, &ns, &ar)
        };
        let params = ListParams::default().fields(&format!("involvedObject.name={name}"));
        let mut ticker = tokio::time::interval(OBJECT_EVENTS_POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_error = String::new();
        loop {
            ticker.tick().await;
            let list = match api.list(&params).await {
                Ok(list) => {
                    last_error.clear();
                    list
                }
                Err(e) => {
                    // a silent empty list would look like "no events"; say why
                    let msg = e.to_string();
                    if msg != last_error {
                        debug_log(&format!("object events list failed: {msg}"));
                        last_error = msg;
                    }
                    continue;
                }
            };
            let now = jiff::Timestamp::now().as_second();
            let mut rows: Vec<EventRow> =
                list.items.into_iter().map(|e| event_row(&e, now)).collect();
            rows.sort_by_key(|e| std::cmp::Reverse(e.at));
            rows.truncate(EVENTS_SHOWN);
            let _ = tx.send(rows);
            egui_ctx.request_repaint();
        }
    })
}

#[cfg(test)]
#[path = "cluster_tests.rs"]
mod tests;
