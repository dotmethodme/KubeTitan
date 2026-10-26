//! Prometheus discovery and pod usage timelines.
//!
//! Mirrors the Wails backend's `prometheus.go`: find the cluster's Prometheus
//! service, then query it **through the Kubernetes service proxy**, so no
//! extra ingress, port-forward or credentials are needed.
//!
//! Only pod timelines are implemented so far; the Vue app also charts
//! workloads, which would need a label-selector query path.

use eframe::egui;
use kube::Client;
use kube::api::{Api, ApiResource, DynamicObject, ListParams};
use kube::core::GroupVersionKind;
use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Charts cover the last hour, sampled every 30s — the Vue drawer's default.
pub const WINDOW_SECONDS: i64 = 3600;
pub const STEP_SECONDS: u64 = 30;
/// How often an open drawer re-queries.
pub const REFRESH: Duration = Duration::from_secs(60);

/// Services whose names never host the query API (`prometheus.go`'s list).
const EXCLUDED_TOKENS: &[&str] = &[
    "node-exporter",
    "kubelet",
    "coredns",
    "operator",
    "alertmanager",
    "grafana",
    "kube-state-metrics",
    "metrics-server",
    "pushgateway",
    "blackbox",
    "loki",
    "tempo",
    "otel",
];

/// Ports that belong to exporters, not to a queryable Prometheus.
const EXCLUDED_PORTS: &[i64] = &[9100, 9153, 10250, 10255, 4194, 2379];

/// A Prometheus service reachable over the API server's service proxy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub namespace: String,
    pub service: String,
    pub port: i64,
}

impl Target {
    pub fn address(&self) -> String {
        format!("{}/{}:{}", self.namespace, self.service, self.port)
    }
}

/// One metric series, oldest sample first.
#[derive(Clone, Debug, Default)]
pub struct Series {
    pub points: Vec<(i64, f64)>,
}

impl Series {
    pub fn latest(&self) -> Option<f64> {
        self.points.last().map(|(_, value)| *value)
    }

    pub fn peak(&self) -> f64 {
        self.points
            .iter()
            .map(|(_, value)| *value)
            .fold(0.0, f64::max)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Timeline {
    pub cpu_cores: Series,
    pub memory_bytes: Series,
    pub disk_bytes: Series,
}

/// Series in a timeline: CPU, memory, disk — Vue's `PrometheusMetrics` charts.
const SERIES: usize = 3;

// ---------------------------------------------------------------------------
// discovery
// ---------------------------------------------------------------------------

fn has_excluded_token(name: &str) -> bool {
    let lower = name.to_lowercase();
    EXCLUDED_TOKENS.iter().any(|token| lower.contains(token))
}

fn looks_like_prometheus(name: &str, namespace: &str, labels: &BTreeMap<String, String>) -> bool {
    let name = name.to_lowercase();
    let namespace = namespace.to_lowercase();
    if name.contains("prometheus") || namespace.contains("monitor") {
        return true;
    }
    labels.iter().any(|(key, value)| {
        let key = key.to_lowercase();
        let value = value.to_lowercase();
        key.contains("prometheus")
            || value.contains("prometheus")
            || (key == "app.kubernetes.io/name" && value.contains("prometheus"))
    })
}

/// Higher score wins; the port is the strongest signal after the name.
fn score(name: &str, namespace: &str, port: i64) -> i32 {
    let mut score = 0;
    if name.to_lowercase().contains("prometheus") {
        score += 50;
    }
    if namespace.to_lowercase().contains("monitor") {
        score += 20;
    }
    if port == 9090 {
        score += 60;
    }
    score
}

/// Find a queryable Prometheus: candidates by name/port, then a cheap `up`
/// probe so we never offer a chart that cannot load.
pub async fn discover(client: &Client) -> Option<Target> {
    let ar =
        ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk("", "v1", "Service"), "services");
    let services: Api<DynamicObject> = Api::all_with(client.clone(), &ar);
    let list = services.list(&ListParams::default()).await.ok()?;

    let mut candidates: Vec<Target> = Vec::new();
    for service in &list.items {
        let name = service.metadata.name.clone().unwrap_or_default();
        let namespace = service.metadata.namespace.clone().unwrap_or_default();
        let empty = BTreeMap::new();
        let labels = service.metadata.labels.as_ref().unwrap_or(&empty);
        if has_excluded_token(&name) || !looks_like_prometheus(&name, &namespace, labels) {
            continue;
        }
        for port in service_ports(service) {
            if EXCLUDED_PORTS.contains(&port) {
                continue;
            }
            candidates.push(Target {
                namespace: namespace.clone(),
                service: name.clone(),
                port,
            });
        }
    }
    candidates.sort_by_key(|target| {
        let target_score = score(&target.service, &target.namespace, target.port);
        (
            -target_score,
            target.namespace.clone(),
            target.service.clone(),
            target.port,
        )
    });

    for candidate in candidates {
        if probe(client, &candidate).await {
            return Some(candidate);
        }
    }
    None
}

fn service_ports(service: &DynamicObject) -> Vec<i64> {
    service
        .data
        .get("spec")
        .and_then(|spec| spec.get("ports"))
        .and_then(|ports| ports.as_array())
        .map(|ports| {
            ports
                .iter()
                .filter_map(|port| port.get("port").and_then(|value| value.as_i64()))
                .collect()
        })
        .unwrap_or_default()
}

async fn probe(client: &Client, target: &Target) -> bool {
    let path = format!("/api/v1/query?query=up&time={}", now());
    match proxy(client, target, &path).await {
        Ok(value) => value.get("status").and_then(|s| s.as_str()) == Some("success"),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// queries
// ---------------------------------------------------------------------------

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// `container!=""`/`container!="POD"` drop the pod sandbox, as kubectl top does.
///
/// Note that `container_fs_usage_bytes` is absent from some cAdvisor
/// configurations (kube-prometheus-stack's default allowlist among them); the
/// chart then honestly reports no samples rather than inventing a figure.
fn pod_queries(namespace: &str, pod: &str) -> [String; SERIES] {
    let selector =
        format!("namespace=\"{namespace}\",pod=\"{pod}\",container!=\"\",container!=\"POD\"");
    [
        format!("sum(rate(container_cpu_usage_seconds_total{{{selector}}}[5m]))"),
        format!("sum(container_memory_working_set_bytes{{{selector}}})"),
        format!("sum(container_fs_usage_bytes{{{selector}}})"),
    ]
}

/// A workload's pods are matched by name regex, like the Wails backend: it
/// resolves the workload's pods and quotes each name.
fn workload_queries(namespace: &str, pods: &[String]) -> [String; SERIES] {
    let regex = pods
        .iter()
        .map(|pod| escape_regex(pod))
        .collect::<Vec<_>>()
        .join("|");
    let selector =
        format!("namespace=\"{namespace}\",pod=~\"{regex}\",container!=\"\",container!=\"POD\"");
    [
        format!("sum(rate(container_cpu_usage_seconds_total{{{selector}}}[5m]))"),
        format!("sum(container_memory_working_set_bytes{{{selector}}})"),
        format!("sum(container_fs_usage_bytes{{{selector}}})"),
    ]
}

/// Quote PromQL regex metacharacters (`regexp.QuoteMeta`).
fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if "\\.+*?()|[]{}^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub async fn pod_timeline(
    client: &Client,
    target: &Target,
    namespace: &str,
    pod: &str,
) -> Result<Timeline, String> {
    timeline_for(client, target, pod_queries(namespace, pod)).await
}

/// Usage of every pod behind a workload (Vue's `GetWorkloadPrometheusTimeline`).
pub async fn workload_timeline(
    client: &Client,
    target: &Target,
    namespace: &str,
    pods: &[String],
) -> Result<Timeline, String> {
    timeline_for(client, target, workload_queries(namespace, pods)).await
}

async fn timeline_for(
    client: &Client,
    target: &Target,
    queries: [String; SERIES],
) -> Result<Timeline, String> {
    let [cpu_query, memory_query, disk_query] = queries;
    let cpu_cores = range(client, target, &cpu_query).await?;
    let memory_bytes = range(client, target, &memory_query).await?;
    let disk_bytes = range(client, target, &disk_query).await?;
    Ok(Timeline {
        cpu_cores,
        memory_bytes,
        disk_bytes,
    })
}

async fn range(client: &Client, target: &Target, query: &str) -> Result<Series, String> {
    let end = now();
    let start = end - WINDOW_SECONDS;
    let path = format!(
        "/api/v1/query_range?query={}&start={start}&end={end}&step={STEP_SECONDS}",
        percent_encode(query)
    );
    let body = proxy(client, target, &path).await?;
    parse_matrix(&body)
}

async fn proxy(
    client: &Client,
    target: &Target,
    path_and_query: &str,
) -> Result<serde_json::Value, String> {
    let path = format!(
        "/api/v1/namespaces/{}/services/{}:{}/proxy{path_and_query}",
        target.namespace, target.service, target.port
    );
    let request = http::Request::get(&path)
        .body(Vec::new())
        .map_err(|e| format!("bad request: {e}"))?;
    client.request(request).await.map_err(|e| e.to_string())
}

/// `{"status":"success","data":{"result":[{"values":[[ts,"0.12"],...]}]}}`
fn parse_matrix(body: &serde_json::Value) -> Result<Series, String> {
    if let Some(message) = body.get("error").and_then(|e| e.as_str()) {
        return Err(message.to_string());
    }
    let samples = body
        .pointer("/data/result/0/values")
        .and_then(|values| values.as_array());
    let Some(samples) = samples else {
        return Ok(Series::default()); // no data yet is not an error
    };
    let points = samples
        .iter()
        .filter_map(|sample| {
            let sample = sample.as_array()?;
            let timestamp = sample.first()?.as_f64()? as i64;
            let value = sample.get(1)?.as_str()?.parse::<f64>().ok()?;
            value.is_finite().then_some((timestamp, value))
        })
        .collect();
    Ok(Series { points })
}

/// Percent-encode a PromQL query for use in a query string.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// poller
// ---------------------------------------------------------------------------

/// What a drawer's usage charts cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    Pod {
        namespace: String,
        pod: String,
    },
    /// Every pod behind a workload, resolved by its label selector.
    Workload {
        namespace: String,
        pods: Vec<String>,
    },
}

/// Keeps a drawer's timeline fresh while it is open. Discovery runs once per
/// cycle on purpose: a cluster that deploys Prometheus later starts working
/// without restarting the app.
pub fn start(
    rt: &tokio::runtime::Runtime,
    client: Client,
    subject: Subject,
    tx: Sender<Result<Timeline, String>>,
    egui_ctx: egui::Context,
) -> JoinHandle<()> {
    rt.spawn(async move {
        let mut ticker = tokio::time::interval(REFRESH);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let result = match discover(&client).await {
                None => Err("no Prometheus found in this cluster".to_string()),
                Some(target) => {
                    let timeline = match &subject {
                        Subject::Pod { namespace, pod } => {
                            pod_timeline(&client, &target, namespace, pod).await
                        }
                        Subject::Workload { namespace, pods } => {
                            workload_timeline(&client, &target, namespace, pods).await
                        }
                    };
                    timeline.map_err(|e| format!("{}: {e}", target.address()))
                }
            };
            if tx.send(result).is_err() {
                return; // the drawer closed
            }
            egui_ctx.request_repaint();
        }
    })
}

#[cfg(test)]
#[path = "prometheus_tests.rs"]
mod tests;
