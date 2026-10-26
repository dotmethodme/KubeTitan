//! Reading the app's object store: which rows a view shows, how the search box
//! filters them, and the dashboard's workload counts.

use super::*;

impl App {
    /// The objects the active view shows, in the order the table draws them:
    /// filtered by the search box, sorted by namespace then name.
    ///
    /// Borrowed, and the order only — no row's cells. The table paints a
    /// screenful at a time and asks for those rows one at a time, so extracting
    /// every row's cells up front is work nobody sees: a thousand-pod cluster
    /// paid for a thousand extractions to paint thirty.
    pub fn ordered(&self, _kind: &Kind) -> Vec<&DynamicObject> {
        let mut objects: Vec<&DynamicObject> = self
            .objects
            .values()
            .filter(|obj| self.matches_query(obj))
            .collect();
        objects.sort_by(|a, b| {
            let (a_ns, b_ns) = (
                a.metadata.namespace.as_deref().unwrap_or(""),
                b.metadata.namespace.as_deref().unwrap_or(""),
            );
            a_ns.cmp(b_ns).then_with(|| {
                a.metadata
                    .name
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.metadata.name.as_deref().unwrap_or(""))
            })
        });
        objects
    }

    /// Row count before the search filter, for the view subtitle.
    pub fn unfiltered_count(&self, _kind: &Kind) -> usize {
        self.objects.len()
    }

    /// Search matches every string in the object, like the Vue app's
    /// `useSearch`, so labels, statuses and IPs are searchable too.
    pub(super) fn matches_query(&self, obj: &DynamicObject) -> bool {
        let q = self.query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let name = obj.metadata.name.clone().unwrap_or_default().to_lowercase();
        let ns = obj
            .metadata
            .namespace
            .clone()
            .unwrap_or_default()
            .to_lowercase();
        if name.contains(&q) || ns.contains(&q) {
            return true;
        }
        deep_contains(&obj.data, &q)
    }

    /// `namespace/name` of the open drawer, matching the table's row keys.
    pub fn selected_key(&self) -> Option<String> {
        self.selection
            .as_ref()
            .map(|s| format!("{}/{}", s.namespace, s.name))
    }

    /// Pod metrics for the pods view, rebuilt from the metrics objects.
    pub(crate) fn metrics(&self) -> MetricsIndex {
        cluster::metrics_index(self.metrics.values())
    }

    /// Counts shown on the dashboard.
    pub fn workload_counts(&self) -> Vec<(String, String, char)> {
        // the pod informer is always on, so pod-level numbers are always live
        let pods = self.pods.len();
        let running = self
            .pods
            .values()
            .filter(|p| {
                p.data
                    .get("status")
                    .and_then(|v| v.get("phase"))
                    .and_then(|v| v.as_str())
                    == Some("Running")
            })
            .count();
        let current = match self.store_kind {
            Some(id) => KINDS_FOR_DASHBOARD
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, label)| label.to_string())
                .unwrap_or_else(|| "resources".to_string()),
            None => "resources".to_string(),
        };
        vec![
            (
                "Pods (cluster-wide)".to_string(),
                pods.to_string(),
                icons::CUBE_OUTLINE,
            ),
            (
                "Pods running".to_string(),
                running.to_string(),
                icons::CHECK_CIRCLE_OUTLINE,
            ),
            (
                "Namespaces".to_string(),
                self.namespaces.len().to_string(),
                icons::FOLDER_OUTLINE,
            ),
            (
                format!("{current} in view"),
                self.objects.len().to_string(),
                icons::DATABASE_OUTLINE,
            ),
        ]
    }
}
/// Kinds worth counting on the dashboard, with their plural labels.
const KINDS_FOR_DASHBOARD: &[(&str, &str)] = &[
    ("pods", "pods"),
    ("deployments", "deployments"),
    ("services", "services"),
    ("configmaps", "configmaps"),
];

/// Recursive substring search over an arbitrary JSON value — mirrors the Vue
/// app's `useSearch`, which matches any string anywhere in the object.
fn deep_contains(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(s) => s.to_lowercase().contains(needle),
        serde_json::Value::Number(n) => n.to_string().contains(needle),
        serde_json::Value::Array(items) => items.iter().any(|v| deep_contains(v, needle)),
        serde_json::Value::Object(map) => map.values().any(|v| deep_contains(v, needle)),
        _ => false,
    }
}
