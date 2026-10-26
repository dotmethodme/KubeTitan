//! An interactive shell inside a container.
//!
//! The shell runs on the cluster: `pods/exec` over a WebSocket, the same upgrade
//! `kubectl exec` performs. What is drawn here is a vt100 screen that the
//! shell's output is parsed into — parsing it, rather than printing the bytes,
//! is what makes a prompt, colours and curses-style output land where they
//! belong instead of scrolling past as escape sequences.
//!
//! The dock owns the keyboard while it has focus: a shell needs every key,
//! including the ones the app uses for its own shortcuts (Esc belongs to vim,
//! not to the drawer). Clicking anywhere else in the app hands the keys back.

use eframe::egui::{self, Align2, Color32, FontId, Key, Modifiers, Sense, Ui, Vec2};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams, TerminalSize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::App;
use crate::app::{Msg, TerminalEvent};
use crate::theme;

/// Lines kept above the live screen.
const SCROLLBACK: usize = 2_000;
/// The shell to open. `sh` is in every image worth shelling into; a container
/// without one says so on the screen, which is more honest than guessing at
/// `bash`, `ash` and the rest.
const SHELL: &str = "/bin/sh";
/// The grid's font. Monospace, at the size the tables use.
const FONT_SIZE: f32 = 12.5;

/// The 16 ANSI colours, taken from the app's own palette so a shell's output
/// looks like part of the app rather than a window from somewhere else.
const ANSI: [Color32; 16] = [
    Color32::from_rgb(0x1a, 0x24, 0x31),
    Color32::from_rgb(0xfc, 0x81, 0x81),
    Color32::from_rgb(0x68, 0xd3, 0x91),
    Color32::from_rgb(0xf6, 0xad, 0x55),
    Color32::from_rgb(0x63, 0xb3, 0xed),
    Color32::from_rgb(0xc9, 0x9b, 0xf0),
    Color32::from_rgb(0x6f, 0xd3, 0xd3),
    Color32::from_rgb(0xe8, 0xed, 0xf4),
    Color32::from_rgb(0x40, 0x4d, 0x5c),
    Color32::from_rgb(0xff, 0xa8, 0xa8),
    Color32::from_rgb(0x9b, 0xe6, 0xbb),
    Color32::from_rgb(0xff, 0xd0, 0x8a),
    Color32::from_rgb(0x9c, 0xd3, 0xff),
    Color32::from_rgb(0xe0, 0xc4, 0xff),
    Color32::from_rgb(0xa8, 0xe6, 0xe6),
    Color32::from_rgb(0xff, 0xff, 0xff),
];

/// One shell, and the screen it is talking to.
pub struct Session {
    pub id: String,
    pub namespace: String,
    pub pod: String,
    pub container: String,
    pub parser: vt100::Parser,
    /// typed bytes, on their way to the container's stdin
    input: Option<mpsc::UnboundedSender<Vec<u8>>>,
    /// the grid's size, on its way to the API's resize channel
    size: Option<mpsc::UnboundedSender<(u16, u16)>>,
    /// the cluster's refusal, or the shell's own complaint, shown on the screen
    pub error: Option<String>,
    pub alive: bool,
    /// lines scrolled back from the live screen
    pub scroll: usize,
    /// the size the shell was last told about, so it is told only on change
    sent: (u16, u16),
}

impl Session {
    pub fn title(&self) -> String {
        format!("{} · {}", self.pod, self.container)
    }

    /// What is on the screen. The control channel reports it and the tests read
    /// it, which is how a shell session is checked without a screenshot.
    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }

    pub(crate) fn write(&self, bytes: Vec<u8>) {
        if let Some(input) = &self.input {
            let _ = input.send(bytes);
        }
    }
}

/// Open a shell in `container`, or focus the one already open on it: two
/// sessions onto the same container would be two prompts fighting over one
/// screen.
pub fn open(app: &mut App, namespace: &str, pod: &str, container: &str) {
    app.terminal_open = true;
    if let Some(session) = app
        .terminals
        .iter()
        .find(|session| session.pod == pod && session.container == container)
    {
        app.terminal_focus = Some(session.id.clone());
        return;
    }

    let id = format!("term{}", app.terminals.len() + 1);
    let (input, input_rx) = mpsc::unbounded_channel();
    let (size, size_rx) = mpsc::unbounded_channel();
    app.terminals.push(Session {
        id: id.clone(),
        namespace: namespace.to_string(),
        pod: pod.to_string(),
        container: container.to_string(),
        parser: vt100::Parser::new(24, 80, SCROLLBACK),
        input: Some(input),
        size: Some(size),
        error: None,
        alive: true,
        scroll: 0,
        sent: (80, 24),
    });
    app.terminal_focus = Some(id.clone());
    spawn(app, id, namespace, pod, container, input_rx, size_rx);
}

/// Close a session: dropping the input channel ends the task, and the task
/// dropping the process closes the WebSocket, which is what stops the shell.
pub fn close(app: &mut App, id: &str) {
    let Some(index) = app.terminals.iter().position(|session| session.id == id) else {
        return;
    };
    app.terminals.remove(index);
    if app.terminal_focus.as_deref() == Some(id) {
        app.terminal_focus = app.terminals.last().map(|session| session.id.clone());
    }
    if app.terminals.is_empty() {
        app.terminal_open = false;
    }
}

/// The exec itself, on the app's runtime: one task per session, ending when the
/// shell does or when the UI drops its end of the channels.
fn spawn(
    app: &mut App,
    id: String,
    namespace: &str,
    pod: &str,
    container: &str,
    mut input_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    mut size_rx: mpsc::UnboundedReceiver<(u16, u16)>,
) {
    let client = app.client.clone();
    let tx = app.tx.clone();
    let (namespace, pod, container) = (
        namespace.to_string(),
        pod.to_string(),
        container.to_string(),
    );
    app.rt.spawn(async move {
        let api: Api<Pod> = Api::namespaced(client, &namespace);
        let params = AttachParams::interactive_tty().container(container.clone());
        let mut process = match api.exec(&pod, [SHELL], &params).await {
            Ok(process) => process,
            Err(error) => {
                let _ = tx.send(Msg::Terminal(TerminalEvent::Failed {
                    id,
                    error: refusal(error),
                }));
                return;
            }
        };

        let mut stdin = process.stdin();
        let mut stdout = process.stdout();
        let mut resize = process.terminal_size();
        let mut buf = vec![0u8; 8192];

        loop {
            tokio::select! {
                read = read_into(&mut stdout, &mut buf) => match read {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        let event = TerminalEvent::Output { id: id.clone(), bytes: buf[..read].to_vec() };
                        if tx.send(Msg::Terminal(event)).is_err() {
                            break;
                        }
                    }
                },
                typed = input_rx.recv() => match typed {
                    Some(bytes) => {
                        let Some(stdin) = stdin.as_mut() else { break };
                        if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                            break;
                        }
                    }
                    // the session was closed from the UI
                    None => break,
                },
                size = size_rx.recv() => {
                    if let (Some((cols, rows)), Some(resize)) = (size, resize.as_mut()) {
                        let _ = resize.try_send(TerminalSize { width: cols, height: rows });
                    }
                }
            }
        }
        let _ = tx.send(Msg::Terminal(TerminalEvent::Ended { id }));
    });
}

/// `exec` only hands out the streams it was asked for, so a missing one ends the
/// session rather than spinning on it.
async fn read_into<R: AsyncRead + Unpin>(
    reader: &mut Option<R>,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    match reader.as_mut() {
        Some(reader) => reader.read(buf).await,
        None => Ok(0),
    }
}

/// A refusal from the cluster, with the one thing worth adding: why it is a
/// refusal at all, since "forbidden" on its own reads like a bug in the app.
fn refusal(error: kube::Error) -> String {
    let text = error.to_string();
    if text.contains("Forbidden") || text.contains("forbidden") {
        format!("{text}\n\nThe service account has no pods/exec permission for this pod.")
    } else {
        text
    }
}

// ---------------------------------------------------------------------------
// the dock
// ---------------------------------------------------------------------------

/// The terminal dock: a tab per shell, the screen below, and the keyboard while
/// it has it.
pub fn dock(app: &mut App, ui_root: &mut Ui) {
    if !app.terminal_open || app.terminals.is_empty() {
        return;
    }
    egui::Panel::bottom("terminal")
        .resizable(true)
        .default_size(260.0)
        .show(ui_root, |ui| panel(app, ui));
}

fn panel(app: &mut App, ui: &mut Ui) {
    let mut to_close: Option<String> = None;
    let labels: Vec<(String, Option<String>)> = app
        .terminals
        .iter()
        .map(|session| (session.title(), None))
        .collect();
    let active = app
        .terminals
        .iter()
        .position(|session| Some(session.id.as_str()) == app.terminal_focus.as_deref())
        .unwrap_or(0);

    ui.horizontal(|ui| {
        let clicked = crate::widgets::tabs(ui, &labels, active);
        if let Some(index) = clicked {
            app.terminal_focus = app.terminals.get(index).map(|session| session.id.clone());
        }
        // the strip is a full-width row, so the close button rides above it
        let rect = ui.max_rect();
        let button = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 22.0, rect.top() + 17.0),
            Vec2::splat(26.0),
        );
        let response = ui.interact(button, ui.id().with("terminal-close"), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(button, 0.0, theme::SURFACE_HOVER);
        }
        ui.painter().text(
            button.center(),
            Align2::CENTER_CENTER,
            crate::icons::CLOSE,
            FontId::proportional(14.0),
            if response.hovered() {
                theme::FG
            } else {
                theme::FG_MUTED
            },
        );
        if response.clicked() {
            to_close = app.terminal_focus.clone();
        }
    });
    if let Some(id) = to_close {
        close(app, &id);
        return;
    }

    let Some(id) = app.terminal_focus.clone() else {
        return;
    };
    let Some(session) = app.terminals.iter_mut().find(|session| session.id == id) else {
        return;
    };
    let ctx = app.egui_ctx.clone();
    screen(session, &ctx, ui);
    // A click inside the grid takes the keyboard; a click anywhere else gives it
    // back, so the app's shortcuts return without having to close the dock.
    if ctx.input(|input| input.pointer.any_click()) {
        let inside = ui
            .ctx()
            .pointer_interact_pos()
            .map(|pos| ui.max_rect().contains(pos))
            .unwrap_or(false);
        app.terminal_keys = inside;
    }
}

/// The grid: one run of text per colour change, so a plain line is one draw and
/// a coloured one is a handful, rather than a draw per cell.
fn screen(session: &mut Session, ctx: &egui::Context, ui: &mut Ui) {
    let font = FontId::monospace(FONT_SIZE);
    let (cell_w, line_h) = ui
        .ctx()
        .fonts_mut(|fonts| (fonts.glyph_width(&font, ' '), fonts.row_height(&font)));
    if cell_w <= 0.0 || line_h <= 0.0 {
        return;
    }

    let avail = ui.available_size();
    let cols = ((avail.x / cell_w).floor() as u16).clamp(20, 500);
    let rows = ((avail.y / line_h).floor() as u16).clamp(4, 200);
    if session.sent != (cols, rows) {
        session.parser.screen_mut().set_size(rows, cols);
        if let Some(size) = &session.size {
            let _ = size.send((cols, rows));
        }
        session.sent = (cols, rows);
    }

    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(cols as f32 * cell_w, rows as f32 * line_h),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::TERM_BG);

    let screen = session.parser.screen_mut();
    // scrolling back is only a different window onto the same screen; vt100
    // clamps the offset, so reading it back keeps ours from running off the end
    screen.set_scrollback(session.scroll);
    session.scroll = screen.scrollback();
    let (screen_rows, screen_cols) = screen.size();
    let (last_row, last_col) = (screen_rows.min(rows), screen_cols.min(cols));

    for row in 0..last_row {
        let mut col = 0;
        while col < last_col {
            let Some(cell) = screen.cell(row, col) else {
                col += 1;
                continue;
            };
            let style = Style::of(cell);
            let start = col;
            let mut text = String::new();
            while col < last_col {
                let Some(cell) = screen.cell(row, col) else {
                    break;
                };
                if Style::of(cell) != style {
                    break;
                }
                text.push_str(cell.contents());
                col += 1;
            }
            if text.is_empty() {
                continue;
            }
            let width = text.chars().count() as f32 * cell_w;
            let origin = egui::pos2(
                rect.left() + start as f32 * cell_w,
                rect.top() + row as f32 * line_h,
            );
            if style.background != theme::TERM_BG {
                painter.rect_filled(
                    egui::Rect::from_min_size(origin, Vec2::new(width, line_h)),
                    0.0,
                    style.background,
                );
            }
            painter.text(
                egui::pos2(origin.x, origin.y + line_h / 2.0),
                Align2::LEFT_CENTER,
                &text,
                font.clone(),
                style.foreground,
            );
            if style.underline {
                painter.line_segment(
                    [
                        egui::pos2(origin.x, origin.y + line_h - 1.0),
                        egui::pos2(origin.x + width, origin.y + line_h - 1.0),
                    ],
                    egui::Stroke::new(1.0, style.foreground),
                );
            }
        }
    }

    // The cursor is a beam, not a block: it sits next to the glyph the shell is
    // about to overwrite rather than hiding it. It blinks, which needs a repaint
    // to come back to.
    let (cursor_row, cursor_col) = screen.cursor_position();
    if session.scroll == 0 && session.alive && cursor_row < last_row && cursor_col < last_col {
        let on = ctx.input(|input| ((input.time * 1.5) as u64).is_multiple_of(2));
        if on {
            let x = rect.left() + cursor_col as f32 * cell_w;
            let y = rect.top() + cursor_row as f32 * line_h;
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(x, y), Vec2::new(1.5, line_h)),
                0.0,
                theme::ACCENT,
            );
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(320));
    }

    if let Some(error) = &session.error {
        let mut y = rect.top() + 4.0;
        for line in error.lines().take(6) {
            painter.text(
                egui::pos2(rect.left() + 6.0, y),
                Align2::LEFT_TOP,
                line,
                FontId::monospace(FONT_SIZE),
                theme::BAD,
            );
            y += line_h;
        }
    }
    if !session.alive && session.error.is_none() {
        painter.text(
            egui::pos2(rect.left() + 6.0, rect.bottom() - line_h),
            Align2::LEFT_BOTTOM,
            "[session ended]",
            FontId::monospace(FONT_SIZE),
            theme::FG_MUTED,
        );
    }

    let response = response.on_hover_cursor(egui::CursorIcon::Text);
    if !session.alive {
        return;
    }
    let ctrl = ctx.input(|input| input.modifiers.ctrl);
    for event in ctx.input(|input| input.events.clone()) {
        match &event {
            // with Ctrl held, a letter is a control code, not text
            egui::Event::Text(text) if !ctrl => session.write(text.as_bytes().to_vec()),
            egui::Event::Paste(text) => session.write(text.as_bytes().to_vec()),
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if let Some(bytes) = key_sequence(*key, *modifiers) {
                    session.write(bytes);
                }
            }
            _ => {}
        }
    }
    let scroll = ctx.input(|input| input.smooth_scroll_delta.y);
    if response.hovered() && scroll != 0.0 {
        let step = (scroll / line_h).round() as i64;
        let next = session.scroll as i64 - step;
        session.scroll = next.clamp(0, SCROLLBACK as i64) as usize;
    }
}

/// What a key sends. Terminals are not keyboards: Enter is a carriage return,
/// Backspace is DEL, and the arrows are escape sequences.
fn key_sequence(key: Key, modifiers: Modifiers) -> Option<Vec<u8>> {
    // with Ctrl held a letter is a control code, but Ctrl+Arrow is still an
    // arrow: the control codes are the letters' alone
    let control = (modifiers.ctrl && !modifiers.alt)
        .then(|| control_byte(key))
        .flatten();
    if let Some(byte) = control {
        return Some(vec![byte]);
    }
    let sequence = match key {
        Key::Enter => "\r",
        Key::Tab => "\t",
        Key::Backspace => "\x7f",
        Key::Escape => "\x1b",
        Key::ArrowUp => "\x1b[A",
        Key::ArrowDown => "\x1b[B",
        Key::ArrowRight => "\x1b[C",
        Key::ArrowLeft => "\x1b[D",
        Key::Home => "\x1b[H",
        Key::End => "\x1b[F",
        Key::PageUp => "\x1b[5~",
        Key::PageDown => "\x1b[6~",
        Key::Insert => "\x1b[2~",
        Key::Delete => "\x1b[3~",
        Key::F1 => "\x1bOP",
        Key::F2 => "\x1bOQ",
        Key::F3 => "\x1bOR",
        Key::F4 => "\x1bOS",
        Key::F5 => "\x1b[15~",
        Key::F6 => "\x1b[17~",
        Key::F7 => "\x1b[18~",
        Key::F8 => "\x1b[19~",
        Key::F9 => "\x1b[20~",
        Key::F10 => "\x1b[21~",
        Key::F11 => "\x1b[23~",
        Key::F12 => "\x1b[24~",
        _ => return None,
    };
    Some(sequence.as_bytes().to_vec())
}

/// Ctrl+letter is the letter's place in the alphabet: ^C is 3, ^D is 4.
fn control_byte(key: Key) -> Option<u8> {
    let letter = match key {
        Key::A => b'a',
        Key::B => b'b',
        Key::C => b'c',
        Key::D => b'd',
        Key::E => b'e',
        Key::F => b'f',
        Key::G => b'g',
        Key::H => b'h',
        Key::I => b'i',
        Key::J => b'j',
        Key::K => b'k',
        Key::L => b'l',
        Key::M => b'm',
        Key::N => b'n',
        Key::O => b'o',
        Key::P => b'p',
        Key::Q => b'q',
        Key::R => b'r',
        Key::S => b's',
        Key::T => b't',
        Key::U => b'u',
        Key::V => b'v',
        Key::W => b'w',
        Key::X => b'x',
        Key::Y => b'y',
        Key::Z => b'z',
        _ => return None,
    };
    Some(letter - b'a' + 1)
}

/// The colours a run of cells shares.
#[derive(PartialEq, Eq)]
struct Style {
    foreground: Color32,
    background: Color32,
    underline: bool,
}

impl Style {
    fn of(cell: &vt100::Cell) -> Self {
        let (mut foreground, mut background) = (
            vt_color(cell.fgcolor(), theme::FG),
            vt_color(cell.bgcolor(), theme::TERM_BG),
        );
        if cell.inverse() {
            std::mem::swap(&mut foreground, &mut background);
        }
        if cell.dim() {
            foreground = foreground.gamma_multiply(0.7);
        }
        Self {
            foreground,
            background,
            underline: cell.underline(),
        }
    }
}

fn vt_color(color: vt100::Color, default: Color32) -> Color32 {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(index) => ANSI[(index & 0x0f) as usize],
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
