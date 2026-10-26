//! The application: state, cluster wiring and the egui update loop.
//!
//! `App` is the single owner of everything the UI reads; the cluster side runs
//! on a tokio runtime in its own tasks and reports back over `mpsc` channels
//! (see [`watch`]). The update loop drains those messages, then paints.

mod actions;
mod dialogs;
mod harness;
mod keys;
mod logs;
mod query;
mod state;
mod watch;
mod yaml;

use crate::kinds::{Action, Kind, MetricsIndex};
use crate::{
    cluster, discovery, drawer, favorites, icons, kinds, palette, picker, portforward, prometheus,
    settings, terminal, ui, views, widgets,
};
use anyhow::Result;
use eframe::egui::{self, Ui};
use kube::Client;
use kube::core::DynamicObject;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use watch::detect_context;
use watch::{forward_namespaces, forward_status};

use crate::notice::{self, Notice};

// ---------------------------------------------------------------------------
// messages from cluster tasks to the UI thread
// ---------------------------------------------------------------------------

/// What a shell session tells the UI thread.
pub enum TerminalEvent {
    /// bytes the shell printed, to be parsed into its screen
    Output { id: String, bytes: Vec<u8> },
    /// the cluster refused the exec: no permission, no such container
    Failed { id: String, error: String },
    /// the shell exited, or the connection went away
    Ended { id: String },
}

pub enum Msg {
    Namespaces(Vec<String>),
    LogLine(String),
    /// A confirmation or failure from `cluster.rs`, tagged so it can be routed
    /// (see `notice.rs`).
    Status(String),
    Yaml(Result<String, String>),
    /// server-side apply outcome: kubectl's sentence, or the failure
    YamlApplied(Result<String, String>),
    /// informer events: the UI thread owns the object maps
    Watch(cluster::Source, cluster::Watch),
    /// dashboard event feed snapshot
    Events(Vec<cluster::EventRow>),
    /// dashboard applications grid: deployments and statefulsets
    Apps(Vec<cluster::AppRow>),
    /// events about the object open in the drawer
    ObjectEvents(Vec<cluster::EventRow>),
    /// a port forward's status changed: a service resolved, or it failed
    Forward(crate::portforward::Update),
    /// something happened in a shell session
    Terminal(TerminalEvent),
    /// the open pod's Prometheus usage timelines
    Timeline(Result<prometheus::Timeline, String>),
    /// every resource type the cluster serves, from discovery
    ResourceTypes(Vec<discovery::ResourceType>),
    /// a command from the agent control channel, with its reply slot
    Control(crate::control::Command, mpsc::Sender<crate::control::Reply>),
}

/// `cluster.rs` reports on one string channel, shared with the log stream; the
/// tags that decide where a line goes live in `notice.rs`.

#[derive(Clone, PartialEq)]
pub enum View {
    Dashboard,
    /// The list of running port forwards.
    PortForwards,
    /// A resource list, keyed by catalog id.
    Resource(&'static str),
}

/// What the command palette is currently listing (Vue's `PaletteMode`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    /// Views, namespaces and the first few contexts — Ctrl+K / Ctrl+P.
    Commands,
    /// Namespaces only — Ctrl+.
    Namespaces,
    /// Kubeconfig contexts only — Ctrl+,.
    Contexts,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DrawerTab {
    General,
    Logs,
    Conditions,
    /// A secret's or configmap's key/value pairs, decoded.
    Data,
    Yaml,
}

/// An edit of a secret's or configmap's values, held between frames so a
/// keystroke does not have to be re-derived: the original values for the diff,
/// and the text as the user is typing it.
pub struct DataEdit {
    pub kind: &'static Kind,
    pub namespace: String,
    pub name: String,
    pub original: Vec<(String, String)>,
    pub values: Vec<(String, String)>,
}

/// The resource currently open in the drawer.
#[derive(Clone)]
pub struct Selection {
    pub kind: &'static Kind,
    pub namespace: String,
    pub name: String,
    /// Shared rather than owned: the drawer paints every frame, and a deep copy
    /// of a resource per frame was pure churn.
    pub object: Arc<DynamicObject>,
}

/// A destructive action awaiting confirmation.
pub struct Pending {
    pub kind: &'static Kind,
    pub namespace: String,
    pub name: String,
    pub action: Action,
    /// scale target (only for `Action::Scale`)
    pub replicas: i64,
    pub current_replicas: i64,
}

pub struct App {
    pub(crate) rt: tokio::runtime::Runtime,
    pub(crate) client: Client,
    pub(crate) egui_ctx: egui::Context,
    pub(crate) tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,

    // ---- navigation ----
    pub view: View,
    pub nav_open: HashMap<String, bool>,

    // ---- header ----
    context: String,
    /// contexts the user can switch to (kubeconfig, or the in-cluster account)
    pub contexts: Vec<String>,
    pub namespaces: Vec<String>,
    /// empty string = all namespaces
    pub namespace: String,

    // ---- resource informer for the active view ----
    objects: HashMap<String, DynamicObject>,
    store_kind: Option<&'static str>,
    watch_task: Option<tokio::task::JoinHandle<()>>,
    pub loading: bool,
    load_started: Instant,

    // ---- always-on pod informer (workload logs) + metrics ----
    pods: HashMap<String, DynamicObject>,
    pods_task: Option<tokio::task::JoinHandle<()>>,
    metrics: HashMap<String, DynamicObject>,
    metrics_task: Option<tokio::task::JoinHandle<()>>,

    // ---- dashboard event feed ----
    pub events: Vec<cluster::EventRow>,
    events_task: Option<tokio::task::JoinHandle<()>>,
    /// dashboard applications grid
    pub apps: Vec<cluster::AppRow>,
    apps_task: Option<tokio::task::JoinHandle<()>>,

    // ---- selected object's events (drawer General tab) ----
    pub object_events: Vec<cluster::EventRow>,
    object_events_task: Option<tokio::task::JoinHandle<()>>,

    // ---- selected pod's usage charts (drawer General tab) ----
    pub timeline: Option<Result<prometheus::Timeline, String>>,
    timeline_task: Option<tokio::task::JoinHandle<()>>,

    // ---- list state ----
    pub query: String,
    /// Ctrl+/ shortcut reference dialog
    pub shortcuts_open: bool,
    /// Ctrl+F: the next frame puts the caret in the view's search box
    pub focus_search: bool,

    // ---- drawer ----
    pub selection: Option<Selection>,
    pub drawer_tab: DrawerTab,
    pub logs: Vec<String>,
    log_tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Whether the log pane follows the tail, and whether it wraps long lines.
    /// Wrapping on is the Vue's default (`useLogs`), and it is the one that
    /// shows the whole line in a narrow drawer; off, the pane scrolls sideways.
    pub log_follow: bool,
    pub log_wrap: bool,
    /// Ask the apiserver to stamp each log line. Off by default, as the Vue's
    /// single-pod `useLogs` has it (`showTimestamps = false`).
    pub log_timestamps: bool,
    /// container whose logs the drawer streams (first one by default)
    pub log_container: Option<String>,
    pub yaml: Option<Result<String, String>>,
    /// The YAML tab's draft while it is being edited (Vue's `yamlDraft`).
    pub yaml_draft: Option<String>,
    /// the "Apply YAML Changes" review dialog is open
    pub yaml_review: bool,
    pub yaml_applying: bool,
    /// the last apply failure, shown inside the review dialog
    pub yaml_error: Option<String>,

    // ---- dialogs ----
    pub pending: Option<Pending>,

    // ---- palette ----
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_sel: usize,
    /// Which list the palette shows: everything (Ctrl+K/P), namespaces (Ctrl+.)
    /// or contexts (Ctrl+,) — the Vue palette's own three modes.
    pub palette_mode: PaletteMode,

    // ---- resource picker / favorites ----
    /// every resource type the cluster serves (filled by discovery)
    pub resource_types: Vec<discovery::ResourceType>,
    /// resource types pinned to the sidebar
    pub favorites: Vec<discovery::ResourceType>,
    /// The favorite currently in hand, while it is being dragged to a new place.
    pub drag_favorite: Option<usize>,
    /// The secret or configmap being edited, if any.
    pub data_edit: Option<DataEdit>,
    pub picker_open: bool,
    pub picker_query: String,
    pub picker_category: Option<String>,

    /// the one thing the app is telling the user right now: a confirmation that
    /// fades, or a failure that waits for them
    pub notice: Option<Notice>,
    /// When something was last copied, so the drawer can say so where the user
    /// is looking: the header's status line sits behind the drawer while it is
    /// open, which is exactly when a copy happens.
    pub copied_at: Option<std::time::Instant>,
    /// Port forwards the user started, and the tasks serving them. The task is
    /// kept so stopping can abort it — a dropped listener would keep accepting.
    pub forwards: Vec<portforward::Forward>,
    pub(crate) forward_tasks: HashMap<String, tokio::task::JoinHandle<()>>,
    pub(crate) forward_seq: u32,
    /// The "new port forward" dialog, when one is open.
    pub forward_dialog: Option<portforward::Dialog>,
    /// Shells open in containers, the dock that shows them, and which one has
    /// the keyboard. `terminal_keys` is separate from `terminal_focus` on
    /// purpose: the dock can be open while a table row still has the shortcuts.
    pub terminals: Vec<terminal::Session>,
    pub terminal_open: bool,
    pub terminal_focus: Option<String>,
    pub terminal_keys: bool,
    /// Width of the resource drawer, dragged by its left edge and remembered
    /// across runs (`Settings::drawer_width`).
    pub drawer_width: f32,
    started: bool,

    // ---- headless screenshot mode (`--shot PATH`) ----
    shot: Option<String>,
    shot_requested: bool,
    shot_started: Instant,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        rt: tokio::runtime::Runtime,
        client: Client,
        tx: mpsc::Sender<Msg>,
        rx: mpsc::Receiver<Msg>,
        egui_ctx: egui::Context,
        shot: Option<String>,
        open_palette: bool,
    ) -> Self {
        cluster::list_namespaces(&rt, client.clone(), forward_namespaces(tx.clone()));
        Self {
            rt,
            client,
            egui_ctx,
            tx,
            rx,
            view: View::Resource("pods"),
            nav_open: HashMap::new(),
            // the context picked last run, else whatever kube resolves to
            context: settings::load().context.unwrap_or_else(detect_context),
            contexts: cluster::context_names(),
            namespaces: Vec::new(),
            // the namespace picked last run, so the app reopens where it was
            namespace: settings::load().namespace,
            objects: HashMap::new(),
            store_kind: None,
            watch_task: None,
            loading: true,
            load_started: Instant::now(),
            pods: HashMap::new(),
            pods_task: None,
            metrics: HashMap::new(),
            metrics_task: None,
            events: Vec::new(),
            events_task: None,
            apps: Vec::new(),
            apps_task: None,
            object_events: Vec::new(),
            object_events_task: None,
            timeline: None,
            timeline_task: None,
            query: String::new(),
            shortcuts_open: false,
            focus_search: false,
            selection: None,
            drawer_tab: DrawerTab::General,
            logs: Vec::new(),
            log_tasks: Vec::new(),
            log_container: None,
            log_follow: true,
            log_wrap: true,
            log_timestamps: false,
            yaml: None,
            yaml_draft: None,
            yaml_review: false,
            yaml_applying: false,
            yaml_error: None,
            pending: None,
            palette_open: open_palette,
            palette_query: String::new(),
            palette_sel: 0,
            palette_mode: PaletteMode::Commands,
            resource_types: Vec::new(),
            favorites: favorites::load(),
            drag_favorite: None,
            data_edit: None,
            picker_open: false,
            picker_query: String::new(),
            picker_category: None,
            notice: None,
            copied_at: None,
            forwards: portforward::load(),
            forward_tasks: HashMap::new(),
            // ids carry on from the ones that came back from the last run, so a
            // new forward cannot take an id a saved one already has
            forward_seq: portforward::highest_id(),
            forward_dialog: None,
            terminals: Vec::new(),
            terminal_open: false,
            terminal_focus: None,
            terminal_keys: false,
            drawer_width: settings::load()
                .drawer_width
                .unwrap_or(crate::drawer::DEFAULT_W),
            started: false,
            shot,
            shot_requested: false,
            shot_started: Instant::now(),
        }
    }

    // -----------------------------------------------------------------------
    // accessors used by ui.rs / views.rs / drawer.rs / palette.rs
    // -----------------------------------------------------------------------

    pub(crate) fn status_tx(&self) -> mpsc::Sender<String> {
        forward_status(self.tx.clone())
    }

    pub(crate) fn context_label(&self) -> &str {
        &self.context
    }

    pub(crate) fn namespace_label(&self) -> String {
        if self.namespace.is_empty() {
            "All Namespaces".to_string()
        } else {
            self.namespace.clone()
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // a confirmation goes away on its own; a failure waits for the user
        if self.notice.as_ref().is_some_and(Notice::expired) {
            self.notice = None;
        }
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Namespaces(names) => {
                    // A namespace saved last run can be gone now: fall back to
                    // "all namespaces" instead of showing an empty view. An
                    // empty answer means the cluster could not be asked at all —
                    // a context that is down, a permission that is missing — and
                    // that is no reason to forget what the user chose. (No real
                    // cluster has zero namespaces.)
                    if !names.is_empty()
                        && !self.namespace.is_empty()
                        && !names.contains(&self.namespace)
                    {
                        self.namespace.clear();
                        self.save_settings();
                        self.on_namespace_changed();
                    }
                    self.namespaces = names;
                }
                Msg::LogLine(line) => {
                    self.logs.push(line);
                    cluster::cap_logs(&mut self.logs);
                }
                // the one place a message from `cluster.rs` becomes something
                // the user sees
                Msg::Status(raw) => {
                    if let Some(notice) = notice::from_wire(&raw) {
                        self.notice = Some(notice);
                    }
                }
                Msg::Yaml(res) => self.yaml = Some(res),
                Msg::YamlApplied(res) => self.on_yaml_applied(res),
                Msg::Watch(source, event) => self.apply_watch(source, event),
                Msg::Events(rows) => self.events = rows,
                Msg::Apps(rows) => self.apps = rows,
                Msg::ObjectEvents(rows) => self.object_events = rows,
                Msg::Timeline(timeline) => self.timeline = Some(timeline),
                Msg::Forward(update) => crate::portforward::apply(self, update),
                Msg::Terminal(event) => self.apply_terminal(event),
                Msg::ResourceTypes(types) => self.resource_types = types,
                Msg::Control(cmd, reply_tx) => {
                    let quitting = matches!(cmd, crate::control::Command::Quit);
                    let answer = crate::control::apply(self, cmd);
                    let _ = reply_tx.send(answer);
                    if quitting {
                        self.egui_ctx
                            .send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }

        if !self.started {
            self.started = true;
            self.restart_informer("pods");
            // favorites are usable before discovery lands: intern them now
            for resource in self.favorites.clone() {
                kinds::custom_kind(&resource);
            }
            self.start_discovery();
            // cluster-wide pods back the dashboard tiles and workload logs
            self.ensure_pods_store();
            self.restart_events();
            self.restart_apps();
        }

        // stop showing the skeleton once data arrives (or after a grace period,
        // so an empty cluster still renders its empty state)
        if self.loading {
            let has_data = !self.objects.is_empty();
            if has_data || self.loading_elapsed() > Duration::from_secs(4) {
                self.loading = false;
            }
        }

        // shortcuts and modal dismissal live in `keys`
        self.handle_keys(ctx);

        if self.shot.is_some() {
            self.handle_shot(ctx);
        }
    }

    fn ui(&mut self, ui_root: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = self.egui_ctx.clone();
        ui::header(self, ui_root);
        ui::notice_banner(self, ui_root);
        ui::sidebar(self, ui_root);
        // the dock takes its height from the content, so it is added before it
        terminal::dock(self, ui_root);
        if self.selection.is_some() {
            drawer::show(self, ui_root);
        }
        views::central(self, ui_root);
        self.dialogs(&ctx);
        self.forward_dialog(&ctx);
        self.yaml_apply_dialog(&ctx);
        widgets::shortcuts_dialog(&ctx, &mut self.shortcuts_open);
        picker::show(self, &ctx);
        palette::show(self, &ctx);
    }
}
