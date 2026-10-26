//! Agent control channel (`--control`): the app's own action layer, driven by
//! JSON lines on stdin instead of a mouse.
//!
//! This is deliberately **not** the AI assistant: no model, no API keys, nothing
//! that talks to a provider. It exists so an external agent — or the UI test
//! harness — can drive the IDE deterministically instead of guessing pixel
//! coordinates and sleeping. Commands reach the same `App` methods a click
//! reaches, so an agent can do nothing a user cannot.
//!
//! Protocol: one JSON object per line on stdin, one JSON reply per line on
//! stdout, in order. `{"cmd":"state"}` is the assertion surface — it reports
//! what the frame currently shows, which is what a test actually wants to know.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::sync::mpsc;

use crate::app::Msg;
use crate::kinds::{self, Action};
use crate::{App, DrawerTab, PaletteMode, View, notice};

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Snapshot of the current view, selection and counts.
    State,
    /// `view`: `dashboard`, or a catalog id such as `deployments`.
    Nav { view: String },
    /// The namespace filter; an empty name selects all namespaces.
    Namespace { name: String },
    /// The drawer's tab: `general`, `logs`, `conditions`, `data` or `yaml`.
    Tab { name: String },
    /// Switch kubeconfig context by name.
    Context { name: String },
    /// Open the command palette, optionally as a namespace or context switcher.
    Palette { mode: String },
    /// Take the recovery the current notice offers.
    Recover,
    /// Put the current notice away.
    Dismiss,
    /// The list view's search box.
    Query { text: String },
    /// Select a row by object key (`namespace/name`).
    Select { key: String },
    /// Dismiss the topmost modal, otherwise close the drawer.
    Close,
    /// A row action, exactly as the row's context menu offers it. The key is
    /// the row's own: `namespace/name`.
    Action { action: String, key: String },
    /// Start a port forward, exactly as the dialog's Start button does.
    Forward {
        namespace: String,
        /// `pod` or `service`.
        target: String,
        name: String,
        /// 0 means Auto, as an empty local port does in the dialog.
        #[serde(default)]
        local_port: u16,
        remote_port: u16,
    },
    /// Stop the port forward with this id.
    StopForward { id: String },
    /// Open a saved port forward again.
    ConnectForward { id: String },
    /// Forget a saved port forward entirely.
    RemoveForward { id: String },
    /// Open a shell in a pod's container, as the row's "Open terminal" does.
    Terminal {
        namespace: String,
        pod: String,
        container: String,
    },
    /// Type into the focused shell. Everything a keyboard can send, as text.
    TerminalInput { text: String },
    /// Close the focused shell.
    CloseTerminal,
    /// Wrap the log pane's long lines, or let it scroll sideways.
    LogWrap { value: bool },
    /// Follow the log tail, or hold the scroll where the user left it.
    LogFollow { value: bool },
    /// Set the resource drawer's width, as the left-edge drag would.
    DrawerWidth { value: f32 },
    /// Stamp each log line. Restarts the stream: the apiserver only stamps what
    /// the request asked it to.
    LogTimestamps { value: bool },
    /// Close the window and exit; the reply lands before the app goes down.
    Quit,
}

/// What the frame currently shows. Everything a test would otherwise squint at
/// a screenshot to read.
#[derive(Debug, Serialize)]
pub struct State {
    pub view: String,
    pub namespace: String,
    pub context: String,
    pub query: String,
    pub loading: bool,
    /// Objects the view holds, before the search filter.
    pub rows: usize,
    /// Rows actually rendered.
    pub filtered: usize,
    pub selection: Option<String>,
    pub drawer_tab: String,
    pub palette_open: bool,
    /// which list the palette shows: commands | namespaces | contexts
    pub palette_mode: String,
    pub namespaces: usize,
    /// contexts this cluster's kubeconfig offers (or the in-cluster account)
    pub contexts: Vec<String>,
    /// The message the app is showing; empty when there is none.
    pub status: String,
    /// none | info | error
    pub notice_kind: String,
    /// the recovery on offer, if any: `reconnect`
    pub notice_fix: Option<String>,
    /// Every port forward the app is running, failed ones included.
    pub forwards: Vec<ForwardState>,
    /// The port-forward dialog, when it is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward_dialog: Option<DialogState>,
    /// How many shells are open, and which of them has the keyboard.
    pub terminals: usize,
    pub terminal_open: bool,
    /// The log pane's two switches.
    pub log_wrap: bool,
    pub log_follow: bool,
    pub log_timestamps: bool,
    /// Width of the resource drawer, as dragged (or defaulted).
    pub drawer_width: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalState>,
}

/// The focused shell, as the control channel reports it: what it is attached to,
/// whether it is still alive, and what is on its screen. The screen is the point
/// — it is how a shell session is asserted without a screenshot.
#[derive(Debug, Serialize)]
pub struct TerminalState {
    pub id: String,
    pub pod: String,
    pub container: String,
    pub alive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// the visible screen, rows joined by newlines
    pub text: String,
}

/// The port-forward dialog's fields, as the control channel reports them.
#[derive(Debug, Serialize)]
pub struct DialogState {
    pub name: String,
    pub namespace: String,
    /// pod | service
    pub target: String,
    /// Empty means Auto.
    pub local_port: String,
    pub remote_port: String,
    /// What the target listens on, as the dialog spells it out.
    pub ports: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One port forward, as the control channel reports it.
#[derive(Debug, Serialize)]
pub struct ForwardState {
    pub id: String,
    pub name: String,
    pub namespace: String,
    /// pod | service
    pub target: String,
    /// The pod the forward actually reaches; a service resolves to one.
    pub pod: String,
    pub local_port: u16,
    pub remote_port: u16,
    /// the port on the pod the tunnel is actually opened to; differs from
    /// `remote_port` when the target is a service
    pub target_port: u16,
    /// active | error
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<State>,
}

/// Run one command on the UI thread. Failures come back as a sentence rather
/// than a panic: an unknown view or a row that has gone away is ordinary.
pub fn apply(app: &mut App, cmd: Command) -> Reply {
    match cmd {
        Command::State => {}
        Command::Nav { view } => {
            let target = match view.as_str() {
                "dashboard" => Some(View::Dashboard),
                "port-forwards" | "port_forwards" => Some(View::PortForwards),
                _ => kinds::by_id(&view).map(|kind| View::Resource(kind.id)),
            };
            match target {
                Some(target) => app.set_view(target),
                None => return failed(format!("unknown view `{view}`")),
            }
        }
        Command::Namespace { name } => app.switch_namespace(name),
        Command::Tab { name } => {
            let tab = match name.as_str() {
                "general" => DrawerTab::General,
                "logs" => DrawerTab::Logs,
                "conditions" => DrawerTab::Conditions,
                "data" => DrawerTab::Data,
                "yaml" => DrawerTab::Yaml,
                other => {
                    return failed(format!("unknown drawer tab: {other}"));
                }
            };
            app.set_drawer_tab(tab);
        }
        Command::Context { name } => {
            if let Err(e) = app.switch_context(name) {
                return failed(e);
            }
        }
        Command::Palette { mode } => {
            let mode = match mode.as_str() {
                "commands" => PaletteMode::Commands,
                "namespaces" => PaletteMode::Namespaces,
                "contexts" => PaletteMode::Contexts,
                other => return failed(format!("unknown palette mode `{other}`")),
            };
            app.open_palette_mode(mode);
        }
        Command::Query { text } => app.query = text,
        Command::Select { key } => match current_kind(app) {
            Some(kind) => app.select(kind, &key),
            None => return failed("`select` needs an open resource list".to_string()),
        },
        Command::Close => app.dismiss_topmost(),
        Command::Recover => app.apply_notice_fix(),
        Command::Dismiss => app.dismiss_notice(),
        // the window is closed by the UI thread, which owns the egui context
        Command::Quit => {}
        Command::Action { action, key } => {
            let Some(action) = parse_action(&action) else {
                return failed(format!("unknown action `{action}`"));
            };
            match current_kind(app) {
                Some(kind) => app.begin_action(kind, &key, action),
                None => return failed("`action` needs an open resource list".to_string()),
            }
        }
        Command::Forward {
            namespace,
            target,
            name,
            local_port,
            remote_port,
        } => {
            let target = match target.as_str() {
                "pod" => crate::portforward::Target::Pod,
                "service" => crate::portforward::Target::Service,
                other => return failed(format!("`{other}` is not a pod or a service")),
            };
            let request = crate::portforward::Request {
                namespace,
                target,
                name,
                local_port,
                remote_port,
            };
            if let Err(error) = crate::portforward::start(app, request) {
                return failed(error);
            }
        }
        Command::StopForward { id } => {
            if let Err(error) = crate::portforward::stop(app, &id) {
                return failed(error);
            }
        }
        Command::ConnectForward { id } => {
            if let Err(error) = crate::portforward::connect(app, &id) {
                return failed(error);
            }
        }
        Command::RemoveForward { id } => {
            if let Err(error) = crate::portforward::remove(app, &id) {
                return failed(error);
            }
        }
        Command::Terminal {
            namespace,
            pod,
            container,
        } => {
            crate::terminal::open(app, &namespace, &pod, &container);
        }
        Command::TerminalInput { text } => {
            // through the same door the keyboard uses
            let Some(session) = focused(app) else {
                return failed("no shell is open".to_string());
            };
            session.write(text.into_bytes());
        }
        Command::CloseTerminal => {
            let Some(id) = app.terminal_focus.clone() else {
                return failed("no shell is open".to_string());
            };
            crate::terminal::close(app, &id);
        }
        Command::LogWrap { value } => app.log_wrap = value,
        Command::LogFollow { value } => app.log_follow = value,
        Command::DrawerWidth { value } => {
            // the drag's own limits, so the harness cannot set a width the UI
            // could never reach
            app.drawer_width = value.clamp(crate::drawer::MIN_W, crate::drawer::MAX_W);
        }
        Command::LogTimestamps { value } => app.set_log_timestamps(value),
    }
    Reply {
        ok: true,
        error: None,
        state: Some(state(app)),
    }
}

/// The kind of the list currently open, if the view is a resource list.
fn current_kind(app: &App) -> Option<&'static kinds::Kind> {
    match app.view {
        View::Resource(id) => kinds::by_id(id),
        View::Dashboard | View::PortForwards => None,
    }
}

fn parse_action(name: &str) -> Option<Action> {
    match name {
        "scale" => Some(Action::Scale),
        "restart" => Some(Action::Restart),
        "forward" => Some(Action::Forward),
        "delete" => Some(Action::Delete),
        _ => None,
    }
}

fn failed(error: String) -> Reply {
    Reply {
        ok: false,
        error: Some(error),
        state: None,
    }
}

/// The shell that has the keyboard, if any.
fn focused(app: &App) -> Option<&crate::terminal::Session> {
    let id = app.terminal_focus.as_ref()?;
    app.terminals.iter().find(|session| &session.id == id)
}

fn state(app: &App) -> State {
    let (rows, filtered) = match current_kind(app) {
        Some(kind) => (app.unfiltered_count(kind), app.ordered(kind).len()),
        None => (0, 0),
    };
    State {
        view: match app.view {
            View::Dashboard => "dashboard".to_string(),
            View::PortForwards => "port-forwards".to_string(),
            View::Resource(id) => id.to_string(),
        },
        namespace: app.namespace_label(),
        context: app.context_label().to_string(),
        query: app.query.clone(),
        loading: app.loading,
        rows,
        filtered,
        selection: app.selected_key(),
        drawer_tab: match app.drawer_tab {
            DrawerTab::General => "general",
            DrawerTab::Logs => "logs",
            DrawerTab::Conditions => "conditions",
            DrawerTab::Data => "data",
            DrawerTab::Yaml => "yaml",
        }
        .to_string(),
        palette_open: app.palette_open,
        palette_mode: match app.palette_mode {
            PaletteMode::Commands => "commands",
            PaletteMode::Namespaces => "namespaces",
            PaletteMode::Contexts => "contexts",
        }
        .to_string(),
        namespaces: app.namespaces.len(),
        contexts: app.contexts.clone(),
        status: app
            .notice
            .as_ref()
            .map(|n| n.text.clone())
            .unwrap_or_default(),
        notice_kind: match app.notice.as_ref().map(|n| n.kind) {
            Some(notice::Kind::Info) => "info",
            Some(notice::Kind::Error) => "error",
            None => "none",
        }
        .to_string(),
        notice_fix: app
            .notice
            .as_ref()
            .and_then(|n| n.fix)
            .map(|fix| fix.label().to_lowercase()),
        forwards: app
            .forwards
            .iter()
            .map(|forward| ForwardState {
                id: forward.id.clone(),
                name: forward.name.clone(),
                namespace: forward.namespace.clone(),
                target: forward.target.wire().to_string(),
                pod: forward.pod.clone(),
                local_port: forward.local_port,
                remote_port: forward.remote_port,
                target_port: forward.target_port,
                status: forward.status.wire().to_string(),
                error: forward.error.clone(),
            })
            .collect(),
        forward_dialog: app.forward_dialog.as_ref().map(|dialog| DialogState {
            name: dialog.name.clone(),
            namespace: dialog.namespace.clone(),
            target: dialog.target.wire().to_string(),
            local_port: dialog.local_port.clone(),
            remote_port: dialog.remote_port.clone(),
            ports: dialog.ports.clone(),
            error: dialog.error.clone(),
        }),
        terminals: app.terminals.len(),
        terminal_open: app.terminal_open,
        log_wrap: app.log_wrap,
        log_follow: app.log_follow,
        log_timestamps: app.log_timestamps,
        drawer_width: app.drawer_width,
        terminal: app
            .terminals
            .iter()
            .find(|session| Some(&session.id) == app.terminal_focus.as_ref())
            .map(|session| TerminalState {
                id: session.id.clone(),
                pod: session.pod.clone(),
                container: session.container.clone(),
                alive: session.alive,
                error: session.error.clone(),
                text: session.text(),
            }),
    }
}

/// Read commands from stdin, hand them to the UI thread, write replies back.
/// The UI thread owns the app, so the channel keeps replies in order.
pub fn spawn_stdio(tx: mpsc::Sender<Msg>) {
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut out = std::io::stdout();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Command>(&line) {
                Ok(cmd) => {
                    let (reply_tx, reply_rx) = mpsc::channel();
                    if tx.send(Msg::Control(cmd, reply_tx)).is_err() {
                        break;
                    }
                    match reply_rx.recv() {
                        Ok(reply) => reply,
                        Err(_) => failed("the app stopped".to_string()),
                    }
                }
                Err(e) => failed(format!("bad command: {e}")),
            };
            let text = serde_json::to_string(&reply).unwrap_or_else(|e| {
                format!("{{\"ok\":false,\"error\":\"could not encode reply: {e}\"}}")
            });
            let _ = writeln!(out, "{text}");
            let _ = out.flush();
        }
    });
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod control_tests;
