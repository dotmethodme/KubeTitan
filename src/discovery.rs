//! Kubernetes API discovery.
//!
//! Lists every resource type the cluster serves — the port's equivalent of the
//! Wails backend's `GetAllResourceTypes` — so the sidebar can browse and
//! favorite custom resources, which the hand-written catalog in `kinds.rs`
//! cannot cover.

use kube::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One discovered resource type; mirrors the backend's `ResourceType`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceType {
    /// plural resource name, e.g. `certificates`
    pub name: String,
    pub kind: String,
    pub group: String,
    pub version: String,
    pub namespaced: bool,
    #[serde(default)]
    pub short_names: Vec<String>,
    /// grouping used by the picker
    pub category: String,
}

impl ResourceType {
    /// Stable id: `group/plural`, or just the plural for core resources. The
    /// sidebar, view and watcher all key off this.
    pub fn id(&self) -> String {
        if self.group.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.group, self.name)
        }
    }

    /// `certificates.cert-manager.io` style API name, shown as the subtitle.
    pub fn api_name(&self) -> String {
        if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}.{}/{}", self.name, self.group, self.version)
        }
    }
}

/// The backend's `categorizeResource`, kept in step with it.
pub fn categorize(group: &str, kind: &str) -> &'static str {
    if group.is_empty() {
        return match kind {
            "Pod" | "ReplicationController" => "Workloads",
            "Service" | "Endpoints" => "Network",
            "PersistentVolume" | "PersistentVolumeClaim" => "Storage",
            "ConfigMap" | "Secret" | "ServiceAccount" => "Config",
            "Node" | "Namespace" | "Event" => "Cluster",
            "LimitRange" | "ResourceQuota" => "Policy",
            _ => "Cluster",
        };
    }
    match group {
        "apps" | "batch" => "Workloads",
        "networking.k8s.io" => "Network",
        "storage.k8s.io" => "Storage",
        "rbac.authorization.k8s.io" => "RBAC",
        "policy" => "Policy",
        "autoscaling" => "Autoscaling",
        _ => "Custom Resources",
    }
}

/// Every listable resource type in the cluster, sorted by category then name
/// (the order the picker groups them in).
pub async fn all(client: &Client) -> Result<Vec<ResourceType>, String> {
    let mut out = Vec::new();

    let core = get_json(client, "/api/v1").await?;
    collect(&mut out, &core, "");

    // `/apis` lists the groups; each one is then resolved at its preferred
    // version so a resource is never reported twice.
    let groups = get_json(client, "/apis").await?;
    for group in groups["groups"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let name = group["name"].as_str().unwrap_or_default();
        let version = group["preferredVersion"]["version"]
            .as_str()
            .or_else(|| group["versions"][0]["version"].as_str())
            .unwrap_or_default();
        if name.is_empty() || version.is_empty() {
            continue;
        }
        // an aggregated API can be listed but unserved: skip it, don't fail
        let Ok(list) = get_json(client, &format!("/apis/{name}/{version}")).await else {
            continue;
        };
        collect(&mut out, &list, name);
    }

    out.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    out.dedup_by(|a, b| a.id() == b.id());
    Ok(out)
}

fn collect(out: &mut Vec<ResourceType>, list: &Value, group: &str) {
    let Some(resources) = list["resources"].as_array() else {
        return;
    };
    let version = list["groupVersion"]
        .as_str()
        .map(|gv| gv.rsplit('/').next().unwrap_or(gv).to_string())
        .unwrap_or_default();

    for resource in resources {
        let Some(name) = resource["name"].as_str() else {
            continue;
        };
        // subresources (`pods/log`) are not listable resource types
        if name.contains('/') {
            continue;
        }
        let listable = resource["verbs"]
            .as_array()
            .map(|verbs| verbs.iter().any(|v| v.as_str() == Some("list")))
            .unwrap_or(false);
        if !listable {
            continue;
        }
        let Some(kind) = resource["kind"].as_str() else {
            continue;
        };
        out.push(ResourceType {
            name: name.to_string(),
            kind: kind.to_string(),
            group: group.to_string(),
            version: version.clone(),
            namespaced: resource["namespaced"].as_bool().unwrap_or(false),
            short_names: resource["shortNames"]
                .as_array()
                .map(|names| {
                    names
                        .iter()
                        .filter_map(|n| n.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            category: categorize(group, kind).to_string(),
        });
    }
}

/// GET a path off the API server root, reusing the client's auth and TLS.
async fn get_json(client: &Client, path: &str) -> Result<Value, String> {
    let request = http::Request::get(path)
        .body(Vec::new())
        .map_err(|e| format!("bad discovery request {path}: {e}"))?;
    client
        .request::<Value>(request)
        .await
        .map_err(|e| format!("discovery {path}: {e}"))
}
