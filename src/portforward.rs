//! Local port forwarding to a pod, over kube's port-forward channel.
//!
//! A forward is a listener on `127.0.0.1` that bridges each accepted connection
//! to a pod. One forwarder is opened *per connection*: kube's `Portforwarder`
//! hands out a port's stream exactly once, so a second connection through the
//! same one would fail — which is what `kubectl port-forward` does too.
//!
//! A service is resolved to a pod first (its selector, a running pod behind it,
//! and the container port its `targetPort` names), because the API only forwards
//! to pods.

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use tokio::net::{TcpListener, TcpStream};

use crate::App;
use crate::config;
use crate::kinds;

const FILE: &str = "forwards.json";

/// What a forward points at. The API forwards to pods either way; a service is
/// resolved to one of its pods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Pod,
    Service,
}

impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Target::Pod => "Pod",
            Target::Service => "Service",
        }
    }

    /// The wire name, for the control channel.
    pub fn wire(self) -> &'static str {
        match self {
            Target::Pod => "pod",
            Target::Service => "service",
        }
    }

    pub fn parse(text: &str) -> Option<Target> {
        match text.to_ascii_lowercase().as_str() {
            "pod" | "pods" => Some(Target::Pod),
            "service" | "services" | "svc" => Some(Target::Service),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Listening and bridging connections.
    Active,
    /// Saved, not listening: the user can connect it again.
    Stopped,
    /// It could not run, and the row says why.
    Error,
}

impl Status {
    pub fn wire(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Stopped => "stopped",
            Status::Error => "error",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Stopped => "stopped",
            Status::Error => "failed",
        }
    }
}

/// A forward, as the view lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forward {
    pub id: String,
    pub namespace: String,
    pub target: Target,
    pub name: String,
    /// The pod actually being forwarded to, once a service has been resolved.
    pub pod: String,
    pub local_port: u16,
    /// The port the user asked for: a service's own port, which is what belongs
    /// in the saved forward and what a reconnect asks for again.
    pub remote_port: u16,
    /// The port on the pod that serves `remote_port`. For a pod target the two
    /// are the same; for a service this is its `targetPort`, and it is what the
    /// tunnel is opened to. Kept apart from `remote_port` so the saved spec can
    /// never record the pod's port where the service's belongs.
    pub target_port: u16,
    pub status: Status,
    pub error: Option<String>,
}

/// What the user asked for.
#[derive(Clone, Debug)]
pub struct Request {
    pub namespace: String,
    pub target: Target,
    pub name: String,
    /// 0 means "whatever is free", the Vue dialog's Auto.
    pub local_port: u16,
    pub remote_port: u16,
}

/// Why a forward could not be started at all.
pub fn start(app: &mut App, request: Request) -> Result<String, String> {
    let (namespace, name) = (request.namespace.clone(), request.name.clone());
    if name.is_empty() {
        return Err("a port forward needs a resource name".to_string());
    }
    if request.remote_port == 0 {
        return Err("a port forward needs a remote port".to_string());
    }

    // Bind here rather than inside the task: it is instant, and it means the view
    // shows the port that was actually taken, or why none could be, without a
    // round trip through the channel.
    let listener = bind(request.local_port)?;
    let local_port = listener
        .local_addr()
        .map_or(request.local_port, |a| a.port());

    let id = app.next_forward_id();
    app.forwards.push(Forward {
        id: id.clone(),
        namespace: namespace.clone(),
        target: request.target,
        name: name.clone(),
        pod: name.clone(),
        local_port,
        remote_port: request.remote_port,
        target_port: request.remote_port,
        status: Status::Active,
        error: None,
    });
    // Remembered as soon as it exists, so a forward survives a restart — stopped,
    // and ready to be connected again.
    save(app);
    spawn(app, id.clone(), request, listener);
    Ok(id)
}

/// Bind the local end, or say why it could not be had.
fn bind(port: u16) -> Result<std::net::TcpListener, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("could not listen on 127.0.0.1:{port}: {e}"))?;
    let _ = listener.set_nonblocking(true);
    Ok(listener)
}

/// Put a bound listener to work: accept connections until the task is aborted.
fn spawn(app: &mut App, id: String, request: Request, listener: std::net::TcpListener) {
    let client = app.client.clone();
    let tx = app.tx.clone();
    let namespace = request.namespace.clone();
    let task_id = id.clone();
    let task = app.rt.spawn(async move {
        let result = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => serve(listener, client, &namespace, &request, &tx, &task_id).await,
            Err(e) => Err(format!("could not use the listener: {e}")),
        };
        if let Err(e) = result {
            let _ = tx.send(crate::app::Msg::Forward(Update::Failed {
                id: task_id,
                error: e,
            }));
        }
    });
    app.forward_tasks.insert(id, task);
}

/// Open a saved forward again, on the port it had: whatever already points at
/// the address keeps working.
pub fn connect(app: &mut App, id: &str) -> Result<(), String> {
    let Some(forward) = app.forwards.iter().find(|f| f.id == id).cloned() else {
        return Err(format!("no port forward `{id}`"));
    };
    // A row that failed can be connected again: the failure may have been the
    // cluster's answer to one attempt, not a fact about the forward.
    if forward.status == Status::Active {
        return Err(format!("{} is already active", label(&forward)));
    }
    let listener = bind(forward.local_port)?;
    let request = Request {
        namespace: forward.namespace.clone(),
        target: forward.target,
        name: forward.name.clone(),
        local_port: forward.local_port,
        remote_port: forward.remote_port,
    };
    if let Some(row) = app.forwards.iter_mut().find(|f| f.id == id) {
        row.status = Status::Active;
        row.error = None;
        row.pod = row.name.clone();
        // the resolution starts over: whatever the last run resolved to is not
        // this run's answer
        row.target_port = row.remote_port;
    }
    spawn(app, id.to_string(), request, listener);
    Ok(())
}

/// What a running forward reports back to the app.
pub enum Update {
    /// The pod a service resolved to, and the port on it.
    Resolved {
        id: String,
        pod: String,
        remote_port: u16,
    },
    /// A connection reached the pod: the forward works.
    Connected { id: String },
    /// The forward stopped working, and why.
    Failed { id: String, error: String },
}

/// Stop listening, and keep the forward in the list: a saved forward is one the
/// user can connect again.
pub fn stop(app: &mut App, id: &str) -> Result<(), String> {
    let Some(task) = app.forward_tasks.remove(id) else {
        return Err(format!("no port forward `{id}`"));
    };
    task.abort();
    if let Some(forward) = app.forwards.iter_mut().find(|f| f.id == id) {
        forward.status = Status::Stopped;
        forward.error = None;
        forward.pod = forward.name.clone();
    }
    save(app);
    Ok(())
}

/// Forget a forward entirely, listener and saved entry both.
pub fn remove(app: &mut App, id: &str) -> Result<(), String> {
    if app.forward_tasks.contains_key(id) {
        stop(app, id)?;
    }
    app.forwards.retain(|f| f.id != id);
    save(app);
    Ok(())
}

/// A forward as it is remembered between runs: what it points at, and the local
/// port it was given. A forward started on Auto keeps the port it got, so the
/// address it was connected on stays the address.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Saved {
    pub id: String,
    pub namespace: String,
    pub target: Target,
    pub name: String,
    pub local_port: u16,
    pub remote_port: u16,
}

impl Forward {
    /// A service resolved to a pod: remember where it went, without touching
    /// what the user asked for. Writing the pod's port over `remote_port` here
    /// is what put a target port into the saved forward.
    pub fn resolved(&mut self, pod: String, target_port: u16) {
        self.pod = pod;
        self.target_port = target_port;
    }

    fn saved(&self) -> Saved {
        Saved {
            id: self.id.clone(),
            namespace: self.namespace.clone(),
            target: self.target,
            name: self.name.clone(),
            local_port: self.local_port,
            remote_port: self.remote_port,
        }
    }
}

impl Saved {
    fn into_forward(self) -> Forward {
        Forward {
            pod: self.name.clone(),
            id: self.id,
            namespace: self.namespace,
            target: self.target,
            name: self.name,
            local_port: self.local_port,
            remote_port: self.remote_port,
            // nothing has resolved yet: the port to open is the one asked for
            target_port: self.remote_port,
            // A saved forward is not running: nothing is listening for it until
            // the user connects it, which is also what keeps a restart from
            // grabbing ports nobody asked for.
            status: Status::Stopped,
            error: None,
        }
    }
}

/// The forwards the user has set up, from the last run.
pub fn load() -> Vec<Forward> {
    let saved: Vec<Saved> = config::load_json(FILE);
    saved.into_iter().map(Saved::into_forward).collect()
}

/// The highest id in the saved list, so this run's forwards cannot collide with
/// it. Read from the file rather than from the loaded rows: the rows are what the
/// app holds, the file is what the next run will read.
pub fn highest_id() -> u32 {
    config::load_json::<Vec<Saved>>(FILE)
        .iter()
        .filter_map(|saved| saved.id.strip_prefix("pf"))
        .filter_map(|number| number.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
}

/// Remember the list as it stands.
pub fn save(app: &mut App) {
    let saved: Vec<Saved> = app.forwards.iter().map(Forward::saved).collect();
    if let Err(message) = config::save_json(FILE, &saved) {
        app.flash_error(message);
    }
}

/// Accept connections for as long as the task lives.
async fn serve(
    listener: TcpListener,
    client: Client,
    namespace: &str,
    request: &Request,
    tx: &Sender<crate::app::Msg>,
    id: &str,
) -> Result<(), String> {
    let (pod, remote_port) = resolve(&client, namespace, request).await?;
    // the pod a service resolved to is worth showing: it is where the bytes go
    let _ = tx.send(crate::app::Msg::Forward(Update::Resolved {
        id: id.to_string(),
        pod: pod.clone(),
        remote_port,
    }));

    loop {
        let (socket, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => return Err(format!("stopped listening: {e}")),
        };
        let client = client.clone();
        let pod = pod.clone();
        let tx = tx.clone();
        let id = id.to_string();
        let namespace = namespace.to_string();
        tokio::spawn(async move {
            if let Err(e) = bridge(socket, client, &namespace, &pod, remote_port, &tx, &id).await {
                let _ = tx.send(crate::app::Msg::Forward(Update::Failed { id, error: e }));
            }
        });
    }
}

/// One connection: a fresh forwarder, its stream, and a two-way copy.
async fn bridge(
    mut socket: TcpStream,
    client: Client,
    namespace: &str,
    pod: &str,
    remote_port: u16,
    tx: &Sender<crate::app::Msg>,
    id: &str,
) -> Result<(), String> {
    // kube's port-forward lives on a typed API: `DynamicObject` does not carry
    // the namespace scope the call requires, so this one call is typed.
    let api = pods_api(&client, namespace);
    let mut forwarder = api
        .portforward(pod, &[remote_port])
        .await
        .map_err(|e| format!("port forward to {pod}: {e}"))?;
    let Some(mut stream) = forwarder.take_stream(remote_port) else {
        return Err(format!(
            "port forward to {pod}: no stream for {remote_port}"
        ));
    };
    // The channel is open, so the forward works: a failure from an earlier
    // connection is stale, and the row should say so.
    let _ = tx.send(crate::app::Msg::Forward(Update::Connected {
        id: id.to_string(),
    }));
    let copy = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
    // the server only reports some failures through the error channel, after the
    // connection is over — that is where "connection refused" arrives
    let error = match forwarder.take_error(remote_port) {
        Some(future) => future.await,
        None => None,
    };
    if let Some(message) = error {
        return Err(format!("{pod}:{remote_port}: {message}"));
    }
    copy.map(|_| ())
        .map_err(|e| format!("{pod}:{remote_port}: {e}"))
}

/// The pod to forward to, and the port on it.
async fn resolve(
    client: &Client,
    namespace: &str,
    request: &Request,
) -> Result<(String, u16), String> {
    match request.target {
        Target::Pod => Ok((request.name.clone(), request.remote_port)),
        Target::Service => {
            let api = kube::api::Api::<kube::core::DynamicObject>::namespaced_with(
                client.clone(),
                namespace,
                &api_resource("services"),
            );
            let service = api
                .get(&request.name)
                .await
                .map_err(|e| format!("service {}: {e}", request.name))?;
            let port = service
                .data
                .pointer("/spec/ports")
                .and_then(|p| p.as_array())
                .and_then(|ports| {
                    ports.iter().find(|p| {
                        p.get("port").and_then(|v| v.as_u64()) == Some(request.remote_port as u64)
                    })
                })
                .ok_or_else(|| {
                    format!(
                        "service {} has no port {}",
                        request.name, request.remote_port
                    )
                })?;
            let selector: Vec<String> = service
                .data
                .pointer("/spec/selector")
                .and_then(|s| s.as_object())
                .map(|map| {
                    map.iter()
                        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
                        .collect()
                })
                .unwrap_or_default();
            if selector.is_empty() {
                return Err(format!("service {} selects nothing", request.name));
            }

            let pods = kube::api::Api::<kube::core::DynamicObject>::namespaced_with(
                client.clone(),
                namespace,
                &api_resource("pods"),
            );
            let params = kube::api::ListParams::default().labels(&selector.join(","));
            let list = pods
                .list(&params)
                .await
                .map_err(|e| format!("pods behind {}: {e}", request.name))?;
            let pod = list
                .items
                .iter()
                .find(|p| {
                    p.data.pointer("/status/phase").and_then(|v| v.as_str()) == Some("Running")
                })
                .ok_or_else(|| format!("no running pod behind {}", request.name))?;
            let pod_name = pod
                .metadata
                .name
                .clone()
                .ok_or_else(|| format!("a pod behind {} has no name", request.name))?;

            // a named targetPort is a container port name, which only the pod knows
            let target_port = match port.get("targetPort") {
                Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0) as u16,
                Some(serde_json::Value::String(wanted)) => container_port(pod, wanted)
                    .ok_or_else(|| format!("{pod_name} has no container port named {wanted}"))?,
                _ => request.remote_port,
            };
            if target_port == 0 {
                return Err(format!("service {} maps to no port", request.name));
            }
            Ok((pod_name, target_port))
        }
    }
}

/// The number of a container port by its name, for a service whose `targetPort`
/// is a name rather than a number.
fn container_port(pod: &kube::core::DynamicObject, wanted: &str) -> Option<u16> {
    pod.data
        .pointer("/spec/containers")?
        .as_array()?
        .iter()
        .flat_map(|c| {
            c.get("ports")
                .and_then(|p| p.as_array())
                .map(Vec::as_slice)
                .unwrap_or_default()
        })
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(wanted))
        .and_then(|p| p.get("containerPort").and_then(|v| v.as_u64()))
        .map(|n| n as u16)
}

fn pods_api(client: &Client, namespace: &str) -> kube::api::Api<Pod> {
    kube::api::Api::namespaced(client.clone(), namespace)
}

fn api_resource(id: &str) -> kube::core::ApiResource {
    let kind = kinds::by_id(id).expect("the catalog knows this kind");
    kube::core::ApiResource::from_gvk_with_plural(
        &kube::core::GroupVersionKind::gvk(kind.group, kind.version, kind.kind),
        kind.plural,
    )
}

/// The label a forward shows for its target: `pod-name:8080` reads better than
/// the parts separately.
pub fn label(forward: &Forward) -> String {
    format!("{}:{}", forward.name, forward.remote_port)
}

/// Fold a running forward's report back into the list.
pub fn apply(app: &mut App, update: Update) {
    match update {
        Update::Resolved {
            id,
            pod,
            remote_port,
        } => {
            if let Some(forward) = app.forwards.iter_mut().find(|f| f.id == id) {
                forward.resolved(pod, remote_port);
            }
        }
        Update::Connected { id } => {
            // A later connection can work where an earlier one did not, so a
            // bridge that opened clears the last failure.
            if let Some(forward) = app.forwards.iter_mut().find(|f| f.id == id) {
                forward.status = Status::Active;
                forward.error = None;
            }
        }
        Update::Failed { id, error } => {
            // A forward that cannot run is not silently gone: it stays in the
            // list, marked, with the reason, until the user stops it. The task
            // is finished, so its handle goes. The reason belongs to the row,
            // not to a banner: a client that keeps retrying would raise one
            // every attempt.
            if let Some(forward) = app.forwards.iter_mut().find(|f| f.id == id) {
                forward.status = Status::Error;
                forward.error = Some(explain(&error));
            }
            app.forward_tasks.remove(&id);
        }
    }
}

/// The cluster's answer in plain words. kube's WebSocket plumbing is not what a
/// user needs to read: what matters is whether they may forward at all.
fn explain(error: &str) -> String {
    if error.contains("403 Forbidden") {
        return "the cluster refused the forward: this account may not create \
                `pods/portforward`"
            .to_string();
    }
    if error.contains("404") {
        return "no such pod or service to forward to".to_string();
    }
    if error.contains("ebSocket") {
        return "the cluster would not open a forwarding channel".to_string();
    }
    error.to_string()
}

/// The "new port forward" dialog: what the user is typing, before it becomes a
/// `Request`. Ports are text because an empty local port means Auto, as in the
/// Vue dialog.
#[derive(Clone, Debug)]
pub struct Dialog {
    pub namespace: String,
    pub target: Target,
    pub name: String,
    pub local_port: String,
    pub remote_port: String,
    /// What the target itself listens on, for the line under the port fields:
    /// a service's `port` and its `targetPort` differ on purpose, and the one to
    /// forward to is the service's own.
    pub ports: String,
    pub error: Option<String>,
}

impl Dialog {
    /// Open for a resource whose name and namespace are already known — the
    /// path a row's "Forward port…" action takes.
    pub fn for_resource(
        namespace: &str,
        target: Target,
        name: &str,
        remote_port: &str,
        ports: &str,
    ) -> Dialog {
        Dialog {
            namespace: namespace.to_string(),
            target,
            name: name.to_string(),
            local_port: String::new(),
            remote_port: remote_port.to_string(),
            ports: ports.to_string(),
            error: None,
        }
    }

    /// The request this dialog describes, or the reason it cannot be one.
    pub fn request(&self) -> Result<Request, String> {
        if self.name.trim().is_empty() {
            return Err("a resource name is required".to_string());
        }
        let remote_port = parse_port(&self.remote_port)
            .ok_or_else(|| format!("`{}` is not a port", self.remote_port.trim()))?;
        if remote_port == 0 {
            return Err("the remote port must be 1-65535".to_string());
        }
        let local_port = if self.local_port.trim().is_empty() {
            0
        } else {
            parse_port(&self.local_port)
                .ok_or_else(|| format!("`{}` is not a port", self.local_port.trim()))?
        };
        Ok(Request {
            namespace: self.namespace.clone(),
            target: self.target,
            name: self.name.trim().to_string(),
            local_port,
            remote_port,
        })
    }
}

fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok()
}

/// Ids, kept in one place so the app's counter and the view agree.
impl App {
    pub(crate) fn next_forward_id(&mut self) -> String {
        self.forward_seq += 1;
        format!("pf{}", self.forward_seq)
    }

    /// Forwards by id, for the control channel's state dump.
    pub fn forwards_by_id(&self) -> HashMap<String, Forward> {
        self.forwards
            .iter()
            .map(|f| (f.id.clone(), f.clone()))
            .collect()
    }
}

#[cfg(test)]
#[path = "portforward_tests.rs"]
mod tests;
