//! Command palette (Ctrl+K) — port of CommandPalette.vue.

use eframe::egui;
use egui::{Align2, Key, Ui, Vec2};

use crate::{App, PaletteMode, View, theme};

// ---- command model ----

/// The navigable views in the order the sidebar lists them: Dashboard, then the
/// catalog's sections, each with the icon and the two labels the palette shows.
/// Ctrl+1-9 counts through exactly this list, so the numbers and the sidebar can
/// never disagree.
pub fn nav_entries() -> Vec<(View, char, &'static str, &'static str)> {
    let mut entries = vec![(
        View::Dashboard,
        crate::icons::DASHBOARD,
        "Dashboard",
        "Overview",
    )];
    for section in crate::kinds::SECTIONS {
        for id in section.kinds {
            let Some(kind) = crate::kinds::by_id(id) else {
                continue;
            };
            entries.push((
                View::Resource(kind.id),
                section.icon, // Vue palette: icon = section icon
                kind.label,
                section.label,
            ));
        }
    }
    entries
}

#[derive(Clone)]
enum Cmd {
    /// Go to a view ("Go to X", subtitle "section")
    Nav(View, char, &'static str, &'static str),
    /// Switch to namespace
    Ns(String),
    /// Show all namespaces
    AllNs,
    /// Switch kubeconfig context
    Ctx(String),
}

fn commands(app: &App, mode: PaletteMode) -> Vec<Cmd> {
    let mut cmds: Vec<Cmd> = nav_entries()
        .into_iter()
        .map(|(view, icon, title, section)| Cmd::Nav(view, icon, title, section))
        .collect();
    // favorited resource types, including custom resources
    for resource in &app.favorites {
        let kind = crate::kinds::custom_kind(resource);
        cmds.push(Cmd::Nav(
            View::Resource(kind.id),
            crate::icons::CUBE_OUTLINE,
            kind.label,
            "Favorites",
        ));
    }
    cmds.push(Cmd::AllNs);
    for ns in &app.namespaces {
        cmds.push(Cmd::Ns(ns.clone()));
    }
    // the plain palette lists only the first few contexts, as the Vue one does
    let shown = match mode {
        PaletteMode::Commands => app.contexts.len().min(3),
        _ => app.contexts.len(),
    };
    for context in app.contexts.iter().take(shown) {
        cmds.push(Cmd::Ctx(context.clone()));
    }
    cmds
}

fn cmd_icon(cmd: &Cmd) -> char {
    match cmd {
        Cmd::Nav(_, icon, _, _) => *icon,
        Cmd::Ns(_) => crate::icons::FOLDER,
        Cmd::AllNs => crate::icons::LAYERS,
        Cmd::Ctx(_) => crate::icons::CLOUD_OUTLINE,
    }
}

fn run(app: &mut App, _ctx: &egui::Context, cmd: &Cmd) {
    match cmd {
        Cmd::Nav(view, _, _, _) => app.set_view(view.clone()),
        Cmd::AllNs => app.switch_namespace(String::new()),
        Cmd::Ns(ns) => app.switch_namespace(ns.clone()),
        Cmd::Ctx(name) => {
            let _ = app.switch_context(name.clone());
        }
    }
}

fn title_of(cmd: &Cmd) -> String {
    match cmd {
        Cmd::Nav(_, _, title, _) => (*title).to_string(),
        Cmd::Ns(ns) => ns.clone(),
        Cmd::AllNs => "All Namespaces".into(),
        Cmd::Ctx(name) => name.clone(),
    }
}

fn subtitle_of(app: &App, cmd: &Cmd) -> String {
    match cmd {
        Cmd::Nav(_, _, _, section) => format!("Go to {section}"),
        Cmd::Ns(ns) => format!("Switch to namespace {ns}"),
        Cmd::AllNs => "Show resources from all namespaces".into(),
        // the Vue palette marks the one you are already on
        Cmd::Ctx(name) if *name == app.context_label() => "Current context".into(),
        Cmd::Ctx(_) => "Switch to context".into(),
    }
}

const VISIBLE_ROWS: usize = 6;
const INNER_SIZE: Vec2 = Vec2::new(600.0, 460.0);
const ROW_H: f32 = 60.0; // Vue: px-4 py-3 + title(20) + subtitle(16)
const SEARCH_H: f32 = 52.0;
const FOOTER_H: f32 = 40.0;

// ---- entry point called from App::ui ----

pub fn show(app: &mut App, ctx: &egui::Context) {
    // opening, closing and Escape are `App::handle_keys`' job
    if !app.palette_open {
        return;
    }

    // Vue: `ns:` and `ctx:` typed in the palette narrow it to that list; Ctrl+.
    // and Ctrl+, open the palette already in that mode
    let raw = app.palette_query.to_lowercase();
    let (mode, query) = match app.palette_mode {
        PaletteMode::Commands => match raw.strip_prefix("ns:") {
            Some(rest) => (PaletteMode::Namespaces, rest.trim().to_string()),
            None => match raw.strip_prefix("ctx:") {
                Some(rest) => (PaletteMode::Contexts, rest.trim().to_string()),
                None => (PaletteMode::Commands, raw),
            },
        },
        mode => (mode, raw),
    };
    let matches: Vec<Cmd> = commands(app, mode)
        .into_iter()
        .filter(|c| {
            let in_group = match mode {
                PaletteMode::Commands => true,
                PaletteMode::Namespaces => matches!(c, Cmd::Ns(_) | Cmd::AllNs),
                PaletteMode::Contexts => matches!(c, Cmd::Ctx(_)),
            };
            in_group
                && (query.is_empty()
                    || title_of(c).to_lowercase().contains(&query)
                    || subtitle_of(app, c).to_lowercase().contains(&query))
        })
        .collect();
    if matches.is_empty() {
        app.palette_sel = 0;
    } else {
        app.palette_sel = app.palette_sel.min(matches.len() - 1);
    }
    let count = matches.len();

    // keyboard
    let nav_up = ctx.input(|i| i.key_pressed(Key::ArrowUp));
    let nav_down = ctx.input(|i| i.key_pressed(Key::ArrowDown));
    let accept = ctx.input(|i| i.key_pressed(Key::Enter));
    if nav_up && count > 0 {
        app.palette_sel = (app.palette_sel + count - 1) % count;
    }
    if nav_down && count > 0 {
        app.palette_sel = (app.palette_sel + 1) % count;
    }

    // no anchor: anchored areas size content by LAST FRAME's measured size
    // (state feedback => infinite growth + centered layout). Use fixed_pos:
    // computed once from the screen rect, content lays out from the top-left.
    let screen = ctx
        .input(|i| i.raw.screen_rect)
        .unwrap_or_else(|| egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0)));
    let box_pos = egui::pos2(
        screen.left() + (screen.width() - INNER_SIZE.x) / 2.0,
        screen.top() + 120.0,
    );
    let modal = egui::Modal::new(egui::Id::new("palette-modal"))
        .backdrop_color(egui::Color32::from_black_alpha(150))
        .area(
            egui::Area::new(egui::Id::new("palette-modal-area"))
                .order(egui::Order::Foreground)
                .fixed_pos(box_pos)
                .layout(egui::Layout::top_down(egui::Align::Min).with_main_align(egui::Align::Min)),
        )
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_SECONDARY)
                .stroke(egui::Stroke::new(1.0, theme::EDGE))
                .inner_margin(egui::Margin::ZERO),
        );

    // Fixed rect inside the modal: all palette content is painted at absolute
    // coordinates from `box` — no nested layouts, nothing to mis-size.
    let response = modal.show(ctx, |ui| {
        ui.set_min_size(INNER_SIZE);
        let box_rect = egui::Rect::from_min_size(ui.cursor().min, INNER_SIZE);
        let close = palette_body(ui, app, ctx, box_rect, &matches, accept);
        if close {
            app.palette_open = false;
        }
    });
    if response.backdrop_response.clicked() {
        app.palette_open = false; // Vue: @click.self="close"
    }
}

/// Where the three bands of the box sit. The palette paints at absolute
/// coordinates, so no layout engine can resize it.
struct Bands {
    search: egui::Rect,
    list: egui::Rect,
    footer: egui::Rect,
}

fn body_bands(box_rect: egui::Rect) -> Bands {
    let search = egui::Rect::from_min_size(box_rect.min, egui::vec2(box_rect.width(), SEARCH_H));
    let list = egui::Rect::from_min_size(
        egui::pos2(box_rect.left(), search.bottom()),
        egui::vec2(box_rect.width(), VISIBLE_ROWS as f32 * ROW_H),
    );
    let footer = egui::Rect::from_min_size(
        egui::pos2(box_rect.left(), box_rect.bottom() - FOOTER_H),
        egui::vec2(box_rect.width(), FOOTER_H),
    );
    Bands {
        search,
        list,
        footer,
    }
}

/// Type-to-filter: the palette holds no focused widget, so it reads the raw
/// text events itself.
fn read_typing(app: &mut App, ctx: &egui::Context) {
    ctx.input(|i| {
        for ev in &i.raw.events {
            match ev {
                egui::Event::Text(t) => {
                    for c in t.chars() {
                        if !c.is_control() {
                            app.palette_query.push(c);
                        }
                    }
                }
                egui::Event::Key {
                    key: Key::Backspace,
                    pressed: true,
                    ..
                } => {
                    app.palette_query.pop();
                }
                _ => {}
            }
        }
    });
}

/// The search band: glyph, the query or its placeholder, the caret, and the ESC
/// chip. Returns true when the chip was clicked.
fn search_band(ui: &mut Ui, app: &App, rect: egui::Rect) -> bool {
    let p = ui.painter().clone();
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, theme::EDGE),
    );
    p.text(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        Align2::LEFT_CENTER,
        crate::icons::MAGNIFY,
        egui::FontId::proportional(16.0),
        theme::FG_MUTED,
    );
    let text_pos = egui::pos2(rect.left() + 40.0, rect.center().y);
    let (text, color) = if app.palette_query.is_empty() {
        (
            match app.palette_mode {
                PaletteMode::Namespaces => "Switch namespace…",
                PaletteMode::Contexts => "Switch context…",
                PaletteMode::Commands => "Search resources or commands…",
            },
            theme::FG_MUTED,
        )
    } else {
        (app.palette_query.as_str(), theme::FG)
    };
    p.text(
        text_pos,
        Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(14.0),
        color,
    );
    // caret after the query text (approximate advance: 8px/char)
    let caret_x = text_pos.x + app.palette_query.chars().count() as f32 * 8.0;
    p.line_segment(
        [
            egui::pos2(caret_x, rect.center().y - 9.0),
            egui::pos2(caret_x, rect.center().y + 9.0),
        ],
        egui::Stroke::new(1.0, theme::FG),
    );
    let esc_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 34.0, rect.center().y),
        egui::vec2(40.0, 24.0),
    );
    p.rect_filled(esc_rect, 0.0, theme::SURFACE_TERTIARY);
    p.text(
        esc_rect.center(),
        Align2::CENTER_CENTER,
        "ESC",
        egui::FontId::proportional(11.0),
        theme::FG_MUTED,
    );
    ui.interact(esc_rect, ui.id().with("palette-esc"), egui::Sense::click())
        .clicked()
}

/// The visible slice of the match list, paged around the cursor. Returns true
/// when a row was clicked and the palette should close.
fn result_rows(
    ui: &mut Ui,
    app: &mut App,
    ctx: &egui::Context,
    list: egui::Rect,
    matches: &[Cmd],
) -> bool {
    let p = ui.painter().clone();
    if matches.is_empty() {
        p.text(
            egui::pos2(list.center().x, list.center().y),
            Align2::CENTER_CENTER,
            "No commands found",
            egui::FontId::proportional(13.0),
            theme::FG_MUTED,
        );
        return false;
    }

    let len = matches.len();
    let start = app
        .palette_sel
        .saturating_sub(VISIBLE_ROWS / 2)
        .min(len.saturating_sub(VISIBLE_ROWS));
    let end = (start + VISIBLE_ROWS).min(len);
    for (slot, idx) in (start..end).enumerate() {
        let selected = idx == app.palette_sel;
        let row_rect = egui::Rect::from_min_size(
            egui::pos2(list.left(), list.top() + slot as f32 * ROW_H),
            egui::vec2(list.width(), ROW_H),
        );
        let resp = ui.interact(
            row_rect,
            ui.id().with(("palette-row", idx)),
            egui::Sense::click(),
        );
        // the palette highlights its cursor exactly like hover (Vue does the same)
        if selected || resp.hovered() {
            p.rect_filled(row_rect, 0.0, theme::SURFACE_HOVER);
        }
        p.text(
            egui::pos2(row_rect.left() + 16.0, row_rect.center().y),
            Align2::LEFT_CENTER,
            cmd_icon(&matches[idx]),
            egui::FontId::proportional(16.0),
            if selected {
                theme::ACCENT
            } else {
                theme::FG_MUTED
            },
        );
        p.text(
            egui::pos2(row_rect.left() + 40.0, row_rect.top() + 14.0),
            Align2::LEFT_TOP,
            title_of(&matches[idx]),
            egui::FontId::proportional(14.0),
            if selected {
                theme::FG
            } else {
                theme::FG_SECONDARY
            },
        );
        p.text(
            egui::pos2(row_rect.left() + 40.0, row_rect.bottom() - 14.0),
            Align2::LEFT_BOTTOM,
            subtitle_of(app, &matches[idx]),
            egui::FontId::proportional(12.0),
            theme::FG_MUTED,
        );
        if selected {
            p.text(
                egui::pos2(row_rect.right() - 24.0, row_rect.center().y),
                Align2::RIGHT_CENTER,
                crate::icons::KEYBOARD_RETURN,
                egui::FontId::proportional(15.0),
                theme::FG_MUTED,
            );
        }
        if resp.clicked() {
            let cmd = matches[idx].clone();
            run(app, ctx, &cmd);
            return true;
        }
    }
    false
}

/// The hint line along the bottom of the box.
fn footer_band(p: &egui::Painter, rect: egui::Rect) {
    p.line_segment(
        [rect.left_top(), rect.right_top()],
        egui::Stroke::new(1.0, theme::EDGE),
    );
    p.text(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        Align2::LEFT_CENTER,
        "up/down navigate   enter select   esc close",
        egui::FontId::proportional(11.0),
        theme::FG_MUTED,
    );
}

/// Inside the fixed box: search row, paged rows, footer.
fn palette_body(
    ui: &mut Ui,
    app: &mut App,
    ctx: &egui::Context,
    box_rect: egui::Rect,
    matches: &[Cmd],
    accept: bool,
) -> bool {
    read_typing(app, ctx);
    let bands = body_bands(box_rect);

    let mut close = search_band(ui, app, bands.search);
    if result_rows(ui, app, ctx, bands.list, matches) {
        close = true;
    }
    // Enter takes the highlighted row
    if accept && let Some(cmd) = matches.get(app.palette_sel).cloned() {
        run(app, ctx, &cmd);
        close = true;
    }
    footer_band(&ui.painter().clone(), bands.footer);

    if std::env::var("KT_DEBUG").is_ok() {
        eprintln!(
            "box={:?} search={:?} list={:?} footer={:?} rows={} query={:?}",
            box_rect,
            bands.search,
            bands.list,
            bands.footer,
            matches.len(),
            app.palette_query
        );
    }
    close
}
