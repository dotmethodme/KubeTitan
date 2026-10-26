//! Resource catalog — the single source of truth for everything the app shows.
//!
//! Sidebar sections, the command palette, list-view tables and the drawer all
//! derive from these entries, so adding a resource type is a one-place edit.
//!
//! Column extractors work on [`DynamicObject`] (untyped Kubernetes objects), so
//! one informer implementation covers core types, workload types and CRDs alike.

use eframe::egui::Color32;
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use serde_json::Value;

use crate::{icons, theme};

// ---------------------------------------------------------------------------
// cell / column model
// ---------------------------------------------------------------------------

/// One rendered table cell. `dot` draws a leading status dot like the Vue
/// app's `.status-dot`, so status reads at a glance without colouring text.
#[derive(Clone, Default)]
pub struct Cell {
    pub text: String,
    pub color: Option<Color32>,
    pub mono: bool,
    pub dot: Option<Color32>,
    /// trailing badge (namespaces phase, secrets type, …)
    pub badge: Option<Color32>,
}

impl Cell {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    /// Monospaced, matching the Vue tables' `font-mono text-sm`.
    pub fn mono(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            mono: true,
            ..Default::default()
        }
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            mono: true,
            color: Some(theme::FG_MUTED),
            ..Default::default()
        }
    }

    /// Coloured monospace text, the table's own font — as every constructor here
    /// but `text` and `badge` uses. Named for the colour because that is the
    /// varying part; the font is the table's.
    pub fn colored(text: impl Into<String>, color: Color32) -> Self {
        Self {
            text: text.into(),
            color: Some(color),
            mono: true,
            ..Default::default()
        }
    }

    /// Coloured monospace text preceded by a status dot.
    pub fn status(text: impl Into<String>, color: Color32) -> Self {
        Self {
            text: text.into(),
            color: Some(color),
            dot: Some(color),
            mono: true,
            ..Default::default()
        }
    }

    /// A standalone status dot with no text (health column).
    pub fn dot(color: Color32) -> Self {
        Self {
            dot: Some(color),
            ..Default::default()
        }
    }

    /// Badge (pill) styled cell, used for phase/type columns.
    pub fn badge(text: impl Into<String>, color: Color32) -> Self {
        Self {
            text: text.into(),
            badge: Some(color),
            ..Default::default()
        }
    }

    pub fn empty() -> Self {
        Cell::default()
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Width {
    /// Fixed pixel width (Vue: `width: 40px`, `80px`, …).
    Fixed(f32),
    /// Takes the remaining space (the name column).
    Fill,
}

pub struct Column {
    pub title: &'static str,
    pub width: Width,
    /// Right-align numeric columns (Vue uses `text-right` for counts).
    pub numeric: bool,
}

impl Column {
    pub const fn fill(title: &'static str) -> Self {
        Self {
            title,
            width: Width::Fill,
            numeric: false,
        }
    }

    pub const fn fixed(title: &'static str, width: f32) -> Self {
        Self {
            title,
            width: Width::Fixed(width),
            numeric: false,
        }
    }

    pub const fn num(title: &'static str, width: f32) -> Self {
        Self {
            title,
            width: Width::Fixed(width),
            numeric: true,
        }
    }
}

/// Everything a column extractor may need besides the object itself.
pub struct RowCtx<'a> {
    /// `metrics.k8s.io` samples keyed by namespace and pod. Only pods are
    /// indexed: the API also serves nodes, and no view extracts them yet.
    pub metrics: &'a MetricsIndex,
}

// ---------------------------------------------------------------------------
// metrics (metrics.k8s.io/v1beta1)
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct MetricsIndex {
    /// Keyed by namespace, then pod: a lookup happens for every pod row on every
    /// frame, and a single `"ns/name"` key would build a string each time.
    pods: std::collections::HashMap<String, std::collections::HashMap<String, (u64, u64)>>,
}

impl MetricsIndex {
    pub fn insert(&mut self, ns: &str, name: &str, cpu_milli: u64, mem_bytes: u64) {
        self.pods
            .entry(ns.to_string())
            .or_default()
            .insert(name.to_string(), (cpu_milli, mem_bytes));
    }

    /// `(cpu, memory)` rendered the way `kubectl top` does.
    pub fn pod(&self, ns: &str, name: &str) -> (String, String) {
        match self.pods.get(ns).and_then(|pods| pods.get(name)) {
            Some((cpu, mem)) => (fmt_cpu(*cpu), fmt_mem(*mem)),
            None => (String::new(), String::new()),
        }
    }
}

/// Rendered the way `kubectl top` does: `0m`, `5m`, `250m`, `1`, `1.5`.
fn fmt_cpu(milli: u64) -> String {
    if milli == 0 {
        return "0m".into();
    }
    if milli < 1000 {
        return format!("{milli}m");
    }
    let cores = milli as f64 / 1000.0;
    if cores.fract() == 0.0 {
        format!("{cores:.0}")
    } else {
        format!("{cores:.1}")
    }
}

fn fmt_mem(bytes: u64) -> String {
    const MI: f64 = 1024.0 * 1024.0;
    let mib = bytes as f64 / MI;
    if mib >= 1024.0 {
        format!("{:.1}Gi", mib / 1024.0)
    } else {
        format!("{mib:.0}Mi")
    }
}

// ---------------------------------------------------------------------------
// actions & logs support
// ---------------------------------------------------------------------------

/// Row actions exposed through the right-click context menu, mirroring the
/// Vue views' `ContextMenuContent` items.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Scale,
    Restart,
    /// Open the port-forward dialog for this resource.
    Forward,
    /// Open a shell in one of this pod's containers.
    Terminal,
    Delete,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Scale => "Scale",
            Action::Restart => "Restart",
            Action::Forward => "Forward port\u{2026}",
            Action::Terminal => "Open terminal",
            Action::Delete => "Delete",
        }
    }

    pub fn icon(self) -> char {
        match self {
            Action::Scale => icons::ARROW_EXPAND_VERTICAL,
            Action::Restart => icons::RESTART,
            Action::Forward => icons::LAN_CONNECT,
            Action::Terminal => icons::CONSOLE,
            Action::Delete => icons::DELETE_OUTLINE,
        }
    }

    pub fn destructive(self) -> bool {
        matches!(self, Action::Delete)
    }
}

/// How the drawer's Logs tab gets its lines.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogsSupport {
    /// The object is a pod: follow its container logs directly.
    Pod,
    /// A workload: aggregate logs of every pod matching `spec.selector`.
    Workload,
    None,
}

// ---------------------------------------------------------------------------
// kind
// ---------------------------------------------------------------------------

pub struct Kind {
    /// Stable id used by the sidebar, palette and view dispatch.
    pub id: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub group: &'static str,
    pub version: &'static str,
    pub plural: &'static str,
    pub namespaced: bool,
    pub icon: char,
    pub columns: &'static [Column],
    pub cells: fn(&DynamicObject, &RowCtx) -> Vec<Cell>,
    pub actions: &'static [Action],
    pub logs: LogsSupport,
}

impl Kind {
    pub fn api_resource(&self) -> ApiResource {
        ApiResource::from_gvk_with_plural(
            &GroupVersionKind::gvk(self.group, self.version, self.kind),
            self.plural,
        )
    }

    /// Name shown in headings, e.g. `3 pods in All Namespaces`.
    pub fn lower(&self) -> String {
        self.label.to_lowercase()
    }
}

pub struct Section {
    pub icon: char,
    pub label: &'static str,
    /// Kind ids, in sidebar order.
    pub kinds: &'static [&'static str],
}

// ---------------------------------------------------------------------------
// catalog
// ---------------------------------------------------------------------------

const CORE: &str = "";
const APPS: &str = "apps";
const BATCH: &str = "batch";
const NETWORKING: &str = "networking.k8s.io";
const POLICY: &str = "policy";

pub const KINDS: &[Kind] = &[
    Kind {
        id: "pods",
        label: "Pods",
        kind: "Pod",
        group: CORE,
        version: "v1",
        plural: "pods",
        namespaced: true,
        icon: icons::CUBE_OUTLINE,
        columns: &[
            Column::fill("Name"),
            Column::num("Ready", 90.0),
            Column::fixed("Status", 150.0),
            Column::num("Restarts", 100.0),
            Column::num("CPU", 90.0),
            Column::num("Memory", 100.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_pods,
        actions: &[Action::Terminal, Action::Forward, Action::Delete],
        logs: LogsSupport::Pod,
    },
    Kind {
        id: "deployments",
        label: "Deployments",
        kind: "Deployment",
        group: APPS,
        version: "v1",
        plural: "deployments",
        namespaced: true,
        icon: icons::HEXAGONS,
        columns: &[
            Column::fixed("", 40.0),
            Column::fill("Name"),
            Column::num("Ready", 100.0),
            Column::num("Up-to-date", 120.0),
            Column::num("Available", 110.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_deployments,
        actions: &[Action::Scale, Action::Restart, Action::Delete],
        logs: LogsSupport::Workload,
    },
    Kind {
        id: "statefulsets",
        label: "StatefulSets",
        kind: "StatefulSet",
        group: APPS,
        version: "v1",
        plural: "statefulsets",
        namespaced: true,
        icon: icons::DATABASE_OUTLINE,
        columns: &[
            Column::fixed("", 40.0),
            Column::fill("Name"),
            Column::num("Ready", 100.0),
            Column::fixed("Service", 220.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_statefulsets,
        actions: &[Action::Scale, Action::Restart, Action::Delete],
        logs: LogsSupport::Workload,
    },
    Kind {
        id: "daemonsets",
        label: "DaemonSets",
        kind: "DaemonSet",
        group: APPS,
        version: "v1",
        plural: "daemonsets",
        namespaced: true,
        icon: icons::SERVER_OUTLINE,
        columns: &[
            Column::fixed("", 40.0),
            Column::fill("Name"),
            Column::num("Desired", 100.0),
            Column::num("Current", 100.0),
            Column::num("Ready", 100.0),
            Column::num("Available", 110.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_daemonsets,
        actions: &[Action::Restart, Action::Delete],
        logs: LogsSupport::Workload,
    },
    Kind {
        id: "jobs",
        label: "Jobs",
        kind: "Job",
        group: BATCH,
        version: "v1",
        plural: "jobs",
        namespaced: true,
        icon: icons::CLOCK_OUTLINE,
        columns: &[
            Column::fixed("", 40.0),
            Column::fill("Name"),
            Column::num("Completions", 130.0),
            Column::num("Duration", 110.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_jobs,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    },
    Kind {
        id: "cronjobs",
        label: "CronJobs",
        kind: "CronJob",
        group: BATCH,
        version: "v1",
        plural: "cronjobs",
        namespaced: true,
        icon: icons::CALENDAR_CLOCK,
        columns: &[
            Column::fixed("", 40.0),
            Column::fill("Name"),
            Column::fixed("Schedule", 150.0),
            Column::num("Last Schedule", 140.0),
            Column::num("Active", 90.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_cronjobs,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    },
    Kind {
        id: "nodes",
        label: "Nodes",
        kind: "Node",
        group: CORE,
        version: "v1",
        plural: "nodes",
        namespaced: false,
        icon: icons::SERVER_OUTLINE,
        columns: &[
            Column::fixed("", 40.0),
            Column::fill("Name"),
            Column::fixed("Roles", 180.0),
            Column::fixed("IP", 160.0),
            Column::fixed("Status", 130.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_nodes,
        actions: &[],
        logs: LogsSupport::None,
    },
    Kind {
        id: "namespaces",
        label: "Namespaces",
        kind: "Namespace",
        group: CORE,
        version: "v1",
        plural: "namespaces",
        namespaced: false,
        icon: icons::FOLDER_OUTLINE,
        columns: &[
            Column::fixed("", 40.0),
            Column::fill("Name"),
            Column::fixed("Status", 130.0),
            Column::fill("Labels"),
            Column::num("Age", 90.0),
        ],
        cells: cells_namespaces,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    },
    Kind {
        id: "configmaps",
        label: "ConfigMaps",
        kind: "ConfigMap",
        group: CORE,
        version: "v1",
        plural: "configmaps",
        namespaced: true,
        icon: icons::COG,
        columns: &[
            Column::fill("Name"),
            Column::num("Data", 90.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_configmaps,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    },
    Kind {
        id: "secrets",
        label: "Secrets",
        kind: "Secret",
        group: CORE,
        version: "v1",
        plural: "secrets",
        namespaced: true,
        icon: icons::KEY_OUTLINE,
        columns: &[
            Column::fill("Name"),
            Column::fixed("Type", 240.0),
            Column::num("Data", 90.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_secrets,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    },
    Kind {
        id: "services",
        label: "Services",
        kind: "Service",
        group: CORE,
        version: "v1",
        plural: "services",
        namespaced: true,
        icon: icons::LAN_CONNECT,
        columns: &[
            Column::fill("Name"),
            Column::fixed("Type", 160.0),
            Column::fixed("Cluster IP", 160.0),
            Column::fill("Ports"),
            Column::num("Age", 90.0),
        ],
        cells: cells_services,
        actions: &[Action::Forward, Action::Delete],
        logs: LogsSupport::None,
    },
    Kind {
        id: "ingresses",
        label: "Ingresses",
        kind: "Ingress",
        group: NETWORKING,
        version: "v1",
        plural: "ingresses",
        namespaced: true,
        icon: icons::CLOUD_OUTLINE,
        columns: &[
            Column::fill("Name"),
            Column::fixed("Class", 160.0),
            Column::fill("Hosts"),
            Column::num("Paths", 80.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_ingresses,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    },
    Kind {
        id: "persistentvolumes",
        label: "Persistent Volumes",
        kind: "PersistentVolume",
        group: CORE,
        version: "v1",
        plural: "persistentvolumes",
        namespaced: false,
        icon: icons::HARDDISK,
        columns: &[
            Column::fill("Name"),
            Column::num("Capacity", 100.0),
            Column::fill("Access Modes"),
            Column::fixed("Reclaim Policy", 140.0),
            Column::fixed("Status", 110.0),
            Column::fill("Claim"),
            Column::fill("Storage Class"),
            Column::num("Age", 90.0),
        ],
        cells: cells_pvs,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    },
    Kind {
        id: "persistentvolumeclaims",
        label: "Claims",
        kind: "PersistentVolumeClaim",
        group: CORE,
        version: "v1",
        plural: "persistentvolumeclaims",
        namespaced: true,
        icon: icons::DATABASE_OUTLINE,
        columns: &[
            Column::fill("Name"),
            Column::fixed("Status", 110.0),
            Column::fill("Volume"),
            Column::num("Capacity", 100.0),
            Column::fill("Access Modes"),
            Column::fill("Storage Class"),
            Column::num("Age", 90.0),
        ],
        cells: cells_pvcs,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    },
    Kind {
        id: "poddisruptionbudgets",
        label: "Pod Disruption Budgets",
        kind: "PodDisruptionBudget",
        group: POLICY,
        version: "v1",
        plural: "poddisruptionbudgets",
        namespaced: true,
        icon: icons::SHIELD_OUTLINE,
        columns: &[
            Column::fixed("", 40.0),
            Column::fill("Name"),
            Column::num("Budget", 110.0),
            Column::num("Current", 100.0),
            Column::num("Desired", 100.0),
            Column::num("Allowed Disruptions", 170.0),
            Column::num("Age", 90.0),
        ],
        cells: cells_pdbs,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    },
];

pub const SECTIONS: &[Section] = &[
    Section {
        icon: icons::HEXAGONS,
        label: "Workloads",
        kinds: &[
            "pods",
            "deployments",
            "statefulsets",
            "daemonsets",
            "jobs",
            "cronjobs",
        ],
    },
    Section {
        icon: icons::MAP_MARKER,
        label: "Discovery",
        kinds: &["nodes", "namespaces"],
    },
    Section {
        icon: icons::COG,
        label: "Configuration",
        kinds: &["configmaps", "secrets"],
    },
    Section {
        icon: icons::HARDDISK,
        label: "Storage",
        kinds: &["persistentvolumes", "persistentvolumeclaims"],
    },
    Section {
        icon: icons::LAN_CONNECT,
        label: "Network",
        kinds: &["services", "ingresses", "poddisruptionbudgets"],
    },
];

/// Columns for a discovered resource type: everything discovery can express.
static GENERIC_COLUMNS: &[Column] = &[
    Column {
        title: "Name",
        width: Width::Fill,
        numeric: false,
    },
    Column {
        title: "Namespace",
        width: Width::Fixed(150.0),
        numeric: false,
    },
    Column {
        title: "Created",
        width: Width::Fixed(80.0),
        numeric: false,
    },
];

/// Kinds for resources discovered at runtime (favorites and the resource
/// picker), interned once per id.
///
/// The catalog above is `const`, so a dynamic kind is leaked: a session holds a
/// handful of them, and `Kind`'s `&'static` fields are exactly what lets the
/// watch, view, palette and drawer paths stay unchanged.
static CUSTOM_KINDS: std::sync::OnceLock<std::sync::Mutex<Vec<&'static Kind>>> =
    std::sync::OnceLock::new();

fn custom_kinds() -> &'static std::sync::Mutex<Vec<&'static Kind>> {
    CUSTOM_KINDS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Intern a discovered resource type as a `Kind` so every existing view path
/// works on it. Returns the already-registered kind for a repeated id.
pub fn custom_kind(resource: &crate::discovery::ResourceType) -> &'static Kind {
    let id = resource.id();
    // A type the catalog already knows keeps the catalog's kind, so a favorite
    // is the same row as the sidebar's own entry: same label, columns, actions.
    if let Some(kind) = by_id(&id) {
        return kind;
    }
    // The catalog keys its kinds by plural alone while a picked resource carries
    // its group, so `apps/deployments` is the catalog's `deployments`.
    if let Some(kind) = by_id(&resource.name)
        && (resource.group.is_empty() || kind.group == resource.group)
    {
        return kind;
    }
    if let Ok(registered) = custom_kinds().lock()
        && let Some(kind) = registered.iter().find(|kind| kind.id == id)
    {
        return kind;
    }

    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    let kind: &'static Kind = Box::leak(Box::new(Kind {
        id: leak(id.clone()),
        label: leak(resource.kind.clone()),
        kind: leak(resource.kind.clone()),
        group: leak(resource.group.clone()),
        version: leak(resource.version.clone()),
        plural: leak(resource.name.clone()),
        namespaced: resource.namespaced,
        icon: icons::CUBE_OUTLINE,
        columns: GENERIC_COLUMNS,
        cells: generic_cells,
        actions: &[Action::Delete],
        logs: LogsSupport::None,
    }));
    if let Ok(mut registered) = custom_kinds().lock() {
        registered.push(kind);
    }
    kind
}

/// Whatever the catalog cannot know about a custom resource: identity, place
/// and age.
fn generic_cells(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    vec![
        Cell::mono(obj.metadata.name.clone().unwrap_or_default()),
        Cell::text(namespace(obj)),
        age_cell(obj),
    ]
}

/// Label for count and empty-state lines: catalog labels are already plural
/// ("Pods"), while a discovered kind is singular ("Certificate"), so those get
/// a loose plural.
pub fn count_label(kind: &Kind) -> String {
    let base = kind.lower();
    if is_builtin(kind.id) || base.ends_with('s') {
        base
    } else {
        format!("{base}s")
    }
}

/// True when the id belongs to the hand-written catalog rather than to a
/// resource discovered at runtime.
pub fn is_builtin(id: &str) -> bool {
    KINDS.iter().any(|kind| kind.id == id)
}

pub fn by_id(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id).or_else(|| {
        custom_kinds()
            .lock()
            .ok()
            .and_then(|registered| registered.iter().find(|kind| kind.id == id).copied())
    })
}

// ---------------------------------------------------------------------------
// shared extractors
// ---------------------------------------------------------------------------

fn at<'a>(obj: &'a DynamicObject, path: &str) -> Option<&'a Value> {
    let mut cur = &obj.data;
    for part in path.split('.') {
        cur = cur.get(part)?;
    }
    (!cur.is_null()).then_some(cur)
}

fn text(obj: &DynamicObject, path: &str) -> String {
    match at(obj, path) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn int(obj: &DynamicObject, path: &str) -> i64 {
    at(obj, path).and_then(Value::as_i64).unwrap_or(0)
}

/// A field the API types as `IntOrString` — a count *or* a percentage — as the
/// text it holds. Reading one as an integer silently turns "25%" into 0.
fn int_or_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn str_at(obj: &DynamicObject, path: &str) -> Option<String> {
    at(obj, path).and_then(Value::as_str).map(str::to_owned)
}

fn name(obj: &DynamicObject) -> String {
    obj.metadata.name.clone().unwrap_or_default()
}

fn namespace(obj: &DynamicObject) -> String {
    obj.metadata.namespace.clone().unwrap_or_default()
}

fn created_sec(obj: &DynamicObject) -> Option<i64> {
    obj.metadata
        .creation_timestamp
        .as_ref()
        .map(|t| t.0.as_second())
}

fn age_cell(obj: &DynamicObject) -> Cell {
    match created_sec(obj) {
        Some(sec) => Cell::muted(human_age(sec)),
        None => Cell::empty(),
    }
}

/// Kubernetes-style age: `45s`, `12m`, `7h`, `109d`.
pub fn human_age(created_sec: i64) -> String {
    let d = (jiff::Timestamp::now().as_second() - created_sec).max(0);
    match d {
        s if s < 60 => format!("{s}s"),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        s => format!("{}d", s / 86_400),
    }
}

/// Health dot: green when fully ready, amber when partly, red when nothing
/// is available — the Vue views use exactly these three states.
fn health_dot(ready: i64, desired: i64) -> Color32 {
    if desired > 0 && ready >= desired {
        theme::OK
    } else if ready > 0 {
        theme::WARN
    } else {
        theme::BAD
    }
}

fn labels_of(obj: &DynamicObject) -> std::collections::BTreeMap<String, String> {
    obj.metadata
        .labels
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect()
}

// ---------------------------------------------------------------------------
// per-kind cell extractors
// ---------------------------------------------------------------------------

fn cells_pods(obj: &DynamicObject, ctx: &RowCtx) -> Vec<Cell> {
    let cs = at(obj, "status.containerStatuses").and_then(Value::as_array);
    let (ready, total, restarts) = match cs {
        Some(list) => (
            list.iter()
                .filter(|c| c.get("ready") == Some(&Value::Bool(true)))
                .count(),
            list.len(),
            list.iter()
                .map(|c| c.get("restartCount").and_then(Value::as_i64).unwrap_or(0))
                .sum::<i64>(),
        ),
        None => (0, 0, 0),
    };

    // Waiting/terminated reasons (CrashLoopBackOff, ImagePullBackOff, …) are far
    // more useful than the bare phase, so they win when present. Read off the
    // same list the counts came from.
    let waiting = cs.and_then(|list| {
        list.iter().find_map(|c| {
            let reason = c.get("state")?.get("waiting")?.get("reason")?.as_str()?;
            Some(reason.to_owned())
        })
    });
    let phase = str_at(obj, "status.phase").unwrap_or_else(|| "Unknown".into());
    let status = waiting.unwrap_or(phase);
    let color = theme::phase_color(&status);

    let (cpu, mem) = ctx.metrics.pod(&namespace(obj), &name(obj));
    vec![
        Cell::mono(name(obj)),
        Cell::mono(format!("{ready}/{total}")),
        Cell::status(status, color),
        Cell::mono(restarts.to_string()),
        Cell::muted(cpu),
        Cell::muted(mem),
        age_cell(obj),
    ]
}

fn cells_deployments(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let desired = int(obj, "spec.replicas");
    let ready = int(obj, "status.readyReplicas");
    vec![
        Cell::dot(health_dot(ready, desired)),
        Cell::mono(name(obj)),
        Cell::mono(format!("{ready}/{desired}")),
        Cell::muted(int(obj, "status.updatedReplicas").to_string()),
        Cell::muted(int(obj, "status.availableReplicas").to_string()),
        age_cell(obj),
    ]
}

fn cells_statefulsets(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let desired = int(obj, "spec.replicas");
    let ready = int(obj, "status.readyReplicas");
    vec![
        Cell::dot(health_dot(ready, desired)),
        Cell::mono(name(obj)),
        Cell::mono(format!("{ready}/{desired}")),
        Cell::muted(text(obj, "spec.serviceName")),
        age_cell(obj),
    ]
}

fn cells_daemonsets(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let desired = int(obj, "status.desiredNumberScheduled");
    let ready = int(obj, "status.numberReady");
    vec![
        Cell::dot(health_dot(ready, desired)),
        Cell::mono(name(obj)),
        Cell::muted(desired.to_string()),
        Cell::muted(int(obj, "status.currentNumberScheduled").to_string()),
        Cell::muted(ready.to_string()),
        Cell::muted(int(obj, "status.numberAvailable").to_string()),
        age_cell(obj),
    ]
}

fn cells_jobs(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let succeeded = int(obj, "status.succeeded");
    let failed = int(obj, "status.failed");
    let completions = int(obj, "spec.completions").max(1);
    // the dot is the whole status column here: the words went with the layout
    // that had a text column for them
    let dot = if failed > 0 {
        theme::BAD
    } else if succeeded >= completions {
        theme::OK
    } else {
        theme::WARN
    };

    // Duration: completed jobs use completionTime - startTime, running jobs
    // measure to now (the Vue view does the same).
    let start = at(obj, "status.startTime")
        .and_then(Value::as_str)
        .and_then(parse_secs);
    let end = at(obj, "status.completionTime")
        .and_then(Value::as_str)
        .and_then(parse_secs);
    let duration = match start {
        Some(s) => {
            let e = end.unwrap_or_else(|| jiff::Timestamp::now().as_second());
            short_duration((e - s).max(0))
        }
        None => String::new(),
    };

    vec![
        Cell::dot(dot),
        Cell::mono(name(obj)),
        Cell::mono(format!("{succeeded}/{completions}")),
        Cell::muted(duration),
        age_cell(obj),
    ]
}

fn cells_cronjobs(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let suspend = at(obj, "spec.suspend") == Some(&Value::Bool(true));
    let active = at(obj, "status.active")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let dot = if suspend {
        theme::FG_MUTED
    } else if active > 0 {
        theme::OK
    } else {
        theme::INFO
    };
    let last = str_at(obj, "status.lastScheduleTime")
        .and_then(|s| parse_secs(&s))
        .map(human_age)
        .unwrap_or_else(|| "never".into());
    vec![
        Cell::dot(dot),
        Cell::mono(name(obj)),
        Cell::mono(text(obj, "spec.schedule")),
        Cell::muted(last),
        Cell::muted(active.to_string()),
        age_cell(obj),
    ]
}

fn cells_nodes(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let ready = at(obj, "status.conditions")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter()
                .find(|c| c.get("type").and_then(Value::as_str) == Some("Ready"))
                .and_then(|c| c.get("status").and_then(Value::as_str))
                .map(|s| s == "True")
        })
        .unwrap_or(false);
    let unschedulable = at(obj, "spec.unschedulable") == Some(&Value::Bool(true));
    let (dot, status) = if unschedulable {
        (theme::WARN, "SchedulingDisabled")
    } else if ready {
        (theme::OK, "Ready")
    } else {
        (theme::BAD, "NotReady")
    };

    let roles: Vec<String> = labels_of(obj)
        .keys()
        .filter_map(|k| k.strip_prefix("node-role.kubernetes.io/"))
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
        .collect();
    let roles = if roles.is_empty() {
        "<none>".into()
    } else {
        roles.join(",")
    };

    let ip = at(obj, "status.addresses")
        .and_then(Value::as_array)
        .and_then(|list| {
            list.iter()
                .find(|a| a.get("type").and_then(Value::as_str) == Some("InternalIP"))
                .and_then(|a| a.get("address").and_then(Value::as_str))
                .map(str::to_owned)
        })
        .unwrap_or_default();

    vec![
        Cell::dot(dot),
        Cell::mono(name(obj)),
        Cell::muted(roles),
        Cell::mono(ip),
        Cell::colored(status, dot),
        age_cell(obj),
    ]
}

fn cells_namespaces(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let phase = str_at(obj, "status.phase").unwrap_or_else(|| "Active".into());
    let color = if phase == "Active" {
        theme::OK
    } else {
        theme::WARN
    };
    let labels = labels_of(obj);
    let summary = if labels.is_empty() {
        "<none>".to_string()
    } else {
        labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    vec![
        Cell::dot(color),
        Cell::mono(name(obj)),
        Cell::badge(phase, color),
        Cell::muted(summary),
        age_cell(obj),
    ]
}

fn cells_configmaps(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let data = at(obj, "data")
        .and_then(Value::as_object)
        .map_or(0, |m| m.len());
    let binary = at(obj, "binaryData")
        .and_then(Value::as_object)
        .map_or(0, |m| m.len());
    vec![
        Cell::mono(name(obj)),
        Cell::muted((data + binary).to_string()),
        age_cell(obj),
    ]
}

fn cells_secrets(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let data = at(obj, "data")
        .and_then(Value::as_object)
        .map_or(0, |m| m.len());
    let ty = text(obj, "type");
    vec![
        Cell::mono(name(obj)),
        Cell::muted(ty),
        Cell::muted(data.to_string()),
        age_cell(obj),
    ]
}

fn cells_services(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let ty = text(obj, "spec.type");
    let ports = at(obj, "spec.ports")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|p| {
                    let port = p.get("port").and_then(Value::as_i64).unwrap_or(0);
                    let proto = p.get("protocol").and_then(Value::as_str).unwrap_or("TCP");
                    let node = p.get("nodePort").and_then(Value::as_i64);
                    match node {
                        Some(n) => format!("{port}:{n}/{proto}"),
                        None => format!("{port}/{proto}"),
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    vec![
        Cell::mono(name(obj)),
        Cell::muted(ty),
        Cell::mono(text(obj, "spec.clusterIP")),
        Cell::mono(ports),
        age_cell(obj),
    ]
}

fn cells_ingresses(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let rules = at(obj, "spec.rules").and_then(Value::as_array);
    let hosts = rules
        .map(|list| {
            list.iter()
                .filter_map(|r| r.get("host").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let paths: usize = rules
        .map(|list| {
            list.iter()
                .map(|r| {
                    r.get("http")
                        .and_then(|h| h.get("paths"))
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len)
                })
                .sum()
        })
        .unwrap_or(0);
    vec![
        Cell::mono(name(obj)),
        Cell::muted(text(obj, "spec.ingressClassName")),
        Cell::muted(if hosts.is_empty() { "*".into() } else { hosts }),
        Cell::muted(paths.to_string()),
        age_cell(obj),
    ]
}

fn cells_pvs(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let modes = at(obj, "spec.accessModes")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let claim = {
        let ns = text(obj, "spec.claimRef.namespace");
        let n = text(obj, "spec.claimRef.name");
        if n.is_empty() {
            String::new()
        } else {
            format!("{ns}/{n}")
        }
    };
    vec![
        Cell::mono(name(obj)),
        Cell::mono(text(obj, "spec.capacity.storage")),
        Cell::muted(short_access_modes(&modes)),
        Cell::muted(text(obj, "spec.persistentVolumeReclaimPolicy")),
        Cell::colored(
            text(obj, "status.phase"),
            theme::phase_color(&text(obj, "status.phase")),
        ),
        Cell::mono(claim),
        Cell::muted(text(obj, "spec.storageClassName")),
        age_cell(obj),
    ]
}

fn cells_pvcs(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let phase = text(obj, "status.phase");
    let modes = at(obj, "spec.accessModes")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    vec![
        Cell::mono(name(obj)),
        Cell::colored(
            if phase.is_empty() {
                "Pending".into()
            } else {
                phase.clone()
            },
            theme::phase_color(&phase),
        ),
        Cell::muted(text(obj, "spec.volumeName")),
        Cell::mono(text(obj, "status.capacity.storage")),
        Cell::muted(short_access_modes(&modes)),
        Cell::muted(text(obj, "spec.storageClassName")),
        age_cell(obj),
    ]
}

fn cells_pdbs(obj: &DynamicObject, _ctx: &RowCtx) -> Vec<Cell> {
    let desired = int(obj, "status.desiredHealthy");
    let current = int(obj, "status.currentHealthy");
    // Both are IntOrString in the API — a count *or* a percentage — and "max 25%"
    // is the shape most real clusters use. Read as integers they came out 0, so
    // the Budget column was empty for every percentage budget.
    let min_available = int_or_string(at(obj, "spec.minAvailable"));
    let max_unavailable = int_or_string(at(obj, "spec.maxUnavailable"));
    let budget = if !min_available.is_empty() {
        format!("min {min_available}")
    } else if !max_unavailable.is_empty() {
        format!("max {max_unavailable}")
    } else {
        String::new()
    };
    let allowed = int(obj, "status.disruptionsAllowed");
    vec![
        Cell::dot(health_dot(current, desired)),
        Cell::mono(name(obj)),
        Cell::muted(budget),
        Cell::mono(current.to_string()),
        Cell::mono(desired.to_string()),
        Cell::muted(allowed.to_string()),
        age_cell(obj),
    ]
}

// ---------------------------------------------------------------------------
// small format helpers
// ---------------------------------------------------------------------------

/// `ReadWriteOnce,ReadWriteMany` → `RWO,RWX` (kubectl's short form).
fn short_access_modes(modes: &str) -> String {
    if modes.is_empty() {
        return String::new();
    }
    modes
        .split(", ")
        .map(|m| match m {
            "ReadWriteOnce" => "RWO",
            "ReadOnlyMany" => "ROX",
            "ReadWriteMany" => "RWX",
            "ReadWriteOncePod" => "RWOP",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn short_duration(secs: i64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3_600 => format!("{}m{}s", s / 60, s % 60),
        s => format!("{}h{}m", s / 3_600, (s % 3_600) / 60),
    }
}

/// RFC3339 (as Kubernetes emits it) → unix seconds.
pub fn parse_secs(rfc3339: &str) -> Option<i64> {
    rfc3339
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|t| t.as_second())
}

#[cfg(test)]
#[path = "kinds_tests.rs"]
mod tests;
