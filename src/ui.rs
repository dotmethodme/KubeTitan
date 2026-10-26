//! Chrome: the top header bar and the left sidebar.
//!
//! Both are derived from the resource catalog, so the sidebar can never drift
//! out of sync with the views that actually exist.

use eframe::egui::{self, RichText, Ui};

use crate::{App, View, icons, kinds, notice, theme, widgets};

pub fn header(app: &mut App, ui: &mut Ui) {
    egui::Panel::top("header")
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_SECONDARY)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .stroke(egui::Stroke::new(1.0, theme::EDGE)),
        )
        .show(ui, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::CLOUD_OUTLINE)
                        .size(14.0)
                        .color(theme::FG_MUTED),
                );
                ui.add_space(2.0);

                // context / namespace, two ghost selects — the Vue header's shape
                let context = app.context_label().to_string();
                ghost_select(ui, "ctx_select", &context, app, |ui, app| {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for name in app.contexts.clone() {
                                let active = name == context;
                                if menu_item(ui, &name, active) {
                                    let _ = app.switch_context(name.clone());
                                }
                            }
                        });
                });
                ui.label(RichText::new("/").size(12.0).color(theme::FG_MUTED));
                let namespace = app.namespace_label().to_string();
                ghost_select(ui, "ns_select", &namespace, app, |ui, app| {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            if menu_item(ui, "All Namespaces", app.namespace.is_empty()) {
                                app.switch_namespace(String::new());
                            }
                            for ns in app.namespaces.clone() {
                                let active = ns == app.namespace;
                                if menu_item(ui, &ns, active) {
                                    app.switch_namespace(ns.clone());
                                }
                            }
                        });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(icons::ACCOUNT)
                            .size(16.0)
                            .color(theme::FG_MUTED),
                    );
                    ui.add_space(4.0);
                    ui.separator();
                    // a confirmation lives here, small and out of the way; a
                    // failure gets the banner below, where it can be read
                    if let Some(text) = app
                        .notice
                        .as_ref()
                        .filter(|n| n.kind == notice::Kind::Info)
                        .map(|n| n.text.clone())
                    {
                        let text = if text.chars().count() > 96 {
                            let head: String = text.chars().take(95).collect();
                            format!("{head}…")
                        } else {
                            text
                        };
                        ui.label(RichText::new(text).size(12.0).color(theme::OK));
                    }
                });
            });
            ui.add_space(3.0);
        });
}

/// A failure gets a banner under the header: the whole sentence, and whatever
/// the app can offer to do about it. Confirmations stay in the header — a banner
/// that comes and goes would shove the page around six times a minute.
pub fn notice_banner(app: &mut App, ui: &mut Ui) {
    let Some(notice) = app.notice.clone() else {
        return;
    };
    if notice.kind != notice::Kind::Error {
        return;
    }
    egui::Panel::top("notice_banner")
        .frame(
            egui::Frame::new()
                .fill(theme::alpha(theme::BAD, 0.10))
                .inner_margin(egui::Margin::symmetric(12, 8))
                .stroke(egui::Stroke::new(1.0, theme::EDGE)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::ALERT).size(14.0).color(theme::BAD));
                ui.add_space(6.0);
                // leave the buttons their room, and let the sentence wrap
                let reserved = if notice.fix.is_some() { 200.0 } else { 60.0 };
                let width = (ui.available_width() - reserved).max(160.0);
                ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&notice.text).size(12.5))
                            .wrap()
                            .selectable(false),
                    );
                });
                let mut dismissed = false;
                let mut recover = false;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close = egui::Button::new(RichText::new(icons::CLOSE).size(13.0))
                        .frame(false)
                        .min_size(egui::vec2(20.0, 20.0));
                    dismissed = ui.add(close).on_hover_text("Dismiss").clicked();
                    if let Some(fix) = notice.fix {
                        recover = ui.button(RichText::new(fix.label()).size(12.5)).clicked();
                    }
                });
                if dismissed {
                    app.dismiss_notice();
                }
                if recover {
                    app.apply_notice_fix();
                }
            });
        });
}

/// Full-width nav row: painted bar + icon + left-aligned label.
/// Active rows use the accent tint exactly like Vue's `.menu-item.active`.
fn nav_row(
    ui: &mut Ui,
    selected: bool,
    icon: Option<char>,
    label: &str,
    size: f32,
    weak: bool,
    height: f32,
) -> egui::Response {
    let (rect, resp) = widgets::row(ui, height);
    nav_row_paint(ui, rect, selected, resp.hovered(), icon, label, size, weak);
    resp
}

/// The same row, but pickable: favorites are dragged to reorder.
fn nav_row_draggable(
    ui: &mut Ui,
    selected: bool,
    icon: Option<char>,
    label: &str,
    size: f32,
    weak: bool,
    height: f32,
) -> egui::Response {
    let (rect, resp) = widgets::row_draggable(ui, height);
    nav_row_paint(ui, rect, selected, resp.hovered(), icon, label, size, weak);
    resp
}

/// One sidebar row's paint, shared by the plain and the draggable entry points so
/// the two can never drift apart.
// one argument per thing the row draws; bundling them would only move the list
#[allow(clippy::too_many_arguments)]
fn nav_row_paint(
    ui: &Ui,
    rect: egui::Rect,
    selected: bool,
    hovered: bool,
    icon: Option<char>,
    label: &str,
    size: f32,
    weak: bool,
) {
    if let Some(fill) = widgets::row_fill(selected, hovered, None) {
        ui.painter().rect_filled(rect, 0.0, fill);
    }

    let color = if selected {
        theme::ACCENT
    } else if weak {
        theme::FG_MUTED
    } else {
        theme::FG_SECONDARY
    };

    let label_x = if let Some(icon) = icon {
        ui.painter().text(
            egui::pos2(rect.left() + 10.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            icon,
            egui::FontId::proportional(size + 2.0),
            if selected {
                theme::ACCENT
            } else {
                theme::FG_MUTED
            },
        );
        rect.left() + 32.0
    } else {
        rect.left() + 12.0
    };
    ui.painter().text(
        egui::pos2(label_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(size),
        color,
    );
}

/// A collapsible section header, matching Vue's sidebar buttons: `px-3 py-2`
/// makes it taller than the `py-1` item rows, hovering changes only the text
/// colour (`hover:text-foreground`, no background), and the leading glyph slot
/// shows the section icon until hover, when it becomes a chevron — rotated
/// down while the section is open, right while collapsed.
fn section_row(ui: &mut Ui, icon: char, label: &str, open: bool) -> egui::Response {
    const HEIGHT: f32 = 32.0; // Vue: py-2 around a 0.75rem label
    let (rect, resp) = widgets::row(ui, HEIGHT);
    section_row_paint(ui, rect, icon, label, open, resp.hovered());
    resp
}

/// A section header with a button on its right — the Favorites header's "add",
/// which used to sit as a row of its own under the list. Returns whether the
/// header was clicked and whether the button was, so the caller can tell a
/// toggle from an add.
fn section_row_action(
    ui: &mut Ui,
    icon: char,
    label: &str,
    open: bool,
    action: char,
    tooltip: &str,
) -> (egui::Response, bool) {
    const HEIGHT: f32 = 32.0;
    let (rect, resp) = widgets::row(ui, HEIGHT);
    section_row_paint(ui, rect, icon, label, open, resp.hovered());

    let button = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 8.0 - 26.0, rect.center().y - 13.0),
        egui::Vec2::splat(26.0),
    );
    let mut clicked = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(button), |ui| {
        clicked = widgets::icon_button(ui, action, theme::FG_MUTED, theme::FG)
            .on_hover_text(tooltip)
            .clicked();
    });
    (resp, clicked)
}

/// A section header's paint, shared by the plain and the buttoned entry points so
/// the two can never drift apart.
fn section_row_paint(
    ui: &Ui,
    rect: egui::Rect,
    icon: char,
    label: &str,
    open: bool,
    hovered: bool,
) {
    let glyph = if hovered {
        if open {
            icons::CHEVRON_DOWN
        } else {
            icons::CHEVRON_RIGHT
        }
    } else {
        icon
    };
    ui.painter().text(
        egui::pos2(rect.left() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        glyph,
        egui::FontId::proportional(13.0),
        theme::FG_MUTED,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 32.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label.to_uppercase(),
        egui::FontId::proportional(11.0),
        if hovered { theme::FG } else { theme::FG_MUTED },
    );
}

/// Vertical gap between two sidebar nav rows, on top of each row's own height.
/// Tighter than the theme-wide 6.0 so the nav reads as one dense list.
const ROW_GAP: f32 = 2.0;

pub fn sidebar(app: &mut App, ui: &mut Ui) {
    egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(224.0) // Tailwind w-56
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_SECONDARY)
                .inner_margin(egui::Margin::symmetric(8, 8))
                .stroke(egui::Stroke::new(1.0, theme::EDGE)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("sidebar_scroll")
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    // Every nav row is one `widgets::row` bar in a vertical
                    // layout, so `item_spacing.y` is the *only* thing between
                    // two rows. Theme-wide it is 6.0 (theme.rs); widen it
                    // inside the sidebar alone so tables and forms keep theirs.
                    ui.spacing_mut().item_spacing.y = ROW_GAP;

                    dashboard_row(app, ui);
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);
                    // Favorites first: the types the user works with, above the
                    // catalog they came from.
                    favorites_section(app, ui);
                    resource_sections(app, ui);
                    port_forwards_section(app, ui);
                });
        });
}

/// The Dashboard entry, above the resource sections.
fn dashboard_row(app: &mut App, ui: &mut Ui) {
    let active = app.view == View::Dashboard;
    if nav_row(
        ui,
        active,
        Some(icons::DASHBOARD),
        "Dashboard",
        14.0,
        false,
        28.0,
    )
    .clicked()
    {
        app.set_view(View::Dashboard);
    }
}

/// One collapsible header per catalog section, with its resource types beneath.
fn resource_sections(app: &mut App, ui: &mut Ui) {
    for section in kinds::SECTIONS {
        // the catalog starts folded: the sidebar opens with favorites, and every
        // section the user opens stays open
        let open = *app
            .nav_open
            .entry(section.label.to_string())
            .or_insert(false);
        if section_row(ui, section.icon, section.label, open).clicked() {
            app.nav_open.insert(section.label.to_string(), !open);
        }
        if open {
            for id in section.kinds {
                let Some(kind) = kinds::by_id(id) else {
                    continue;
                };
                let active = app.view == View::Resource(kind.id);
                if nav_row(ui, active, None, kind.label, 13.0, false, 28.0).clicked() {
                    app.set_view(View::Resource(kind.id));
                }
            }
        }
        ui.add_space(4.0);
    }
}

/// The Port Forwards entry, with the forwards running beneath it — the Vue
/// sidebar lists its active sessions the same way.
fn port_forwards_section(app: &mut App, ui: &mut Ui) {
    let active = app.view == View::PortForwards;
    if nav_row(
        ui,
        active,
        Some(icons::LAN_CONNECT),
        "Port Forwards",
        13.0,
        false,
        28.0,
    )
    .clicked()
    {
        app.set_view(View::PortForwards);
    }

    let mut stopped: Option<String> = None;
    for forward in &app.forwards {
        let (rect, response) = widgets::row(ui, 24.0);
        let color = match forward.status {
            crate::portforward::Status::Active => theme::OK,
            crate::portforward::Status::Stopped => theme::FG_MUTED,
            crate::portforward::Status::Error => theme::BAD,
        };
        ui.painter()
            .circle_filled(egui::pos2(rect.left() + 22.0, rect.center().y), 3.5, color);
        ui.painter().text(
            egui::pos2(rect.left() + 34.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!(
                "{} \u{2192} {}",
                crate::portforward::label(forward),
                forward.local_port
            ),
            egui::FontId::monospace(11.0),
            theme::FG_SECONDARY,
        );
        let id = forward.id.clone();
        response.context_menu(|ui| {
            if ui.button("Stop port forward").clicked() {
                stopped = Some(id.clone());
                ui.close();
            }
        });
    }
    if let Some(id) = stopped {
        let _ = crate::portforward::stop(app, &id);
    }
    ui.add_space(4.0);
}

/// The types pinned from the resource picker, including custom resources the
/// catalog does not know: a row's context menu removes it, and the last row
/// opens the picker.
fn favorites_section(app: &mut App, ui: &mut Ui) {
    let open = *app.nav_open.entry("Favorites".to_string()).or_insert(true);
    let (header, add) = section_row_action(
        ui,
        icons::CUBE_OUTLINE,
        "Favorites",
        open,
        icons::PLUS,
        "Add a resource to favorites",
    );
    if add {
        app.picker_open = true;
        app.picker_query.clear();
        app.picker_category = None;
    } else if header.clicked() {
        app.nav_open.insert("Favorites".to_string(), !open);
    }
    if !open {
        return;
    }

    let favorites = app.favorites.clone();
    let mut rows: Vec<egui::Rect> = Vec::with_capacity(favorites.len());
    let mut open_kind = None;
    let mut remove = None;
    let mut dropped = None;
    for (index, resource) in favorites.iter().enumerate() {
        let kind = kinds::custom_kind(resource);
        let response = nav_row_draggable(
            ui,
            app.view == View::Resource(kind.id),
            None,
            kind.label,
            13.0,
            false,
            28.0,
        )
        .on_hover_text(resource.api_name());
        rows.push(response.rect);
        if response.drag_started() {
            app.drag_favorite = Some(index);
        }
        if response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if response.drag_stopped() {
            dropped = Some(index);
        }
        let clicked = response.clicked();
        response.context_menu(|ui| {
            if ui.button("Remove from favorites").clicked() {
                remove = Some(resource.id());
            }
        });
        if clicked {
            open_kind = Some(kind.id);
        }
    }

    // A favorite in hand: show the boundary it would land on, and let it go.
    if let Some(from) = app.drag_favorite {
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            let to = insertion_index(&rows, pointer.y);
            draw_drop_line(ui, &rows, to);
            if dropped == Some(from) {
                app.move_favorite(from, to);
            }
        }
        if dropped == Some(from) {
            app.drag_favorite = None;
        }
    }
    if let Some(id) = open_kind {
        app.set_view(View::Resource(id));
    }
    if let Some(id) = remove {
        app.remove_favorite(&id);
    }
}

/// Which boundary a dragged favorite is nearest: it lands where the row under the
/// pointer starts, or at the end of the list.
fn insertion_index(rows: &[egui::Rect], pointer_y: f32) -> usize {
    rows.iter()
        .position(|rect| pointer_y < rect.center().y)
        .unwrap_or(rows.len())
}

/// The line that shows where a dragged favorite will land.
fn draw_drop_line(ui: &Ui, rows: &[egui::Rect], at: usize) {
    let Some(first) = rows.first() else {
        return;
    };
    let y = match rows.get(at) {
        Some(rect) => rect.top(),
        None => rows.last().map_or(first.top(), |rect| rect.bottom()),
    };
    let line = egui::Rect::from_min_max(
        egui::pos2(first.left() + 6.0, y - 1.0),
        egui::pos2(first.right() - 6.0, y + 1.0),
    );
    ui.painter().rect_filled(line, 0.0, theme::ACCENT);
}

/// A borderless "ghost" select, the shape the Vue header uses: small mono text
/// and a chevron, a hover fill, and no border until it is open.
fn ghost_select(
    ui: &mut Ui,
    id: &str,
    label: &str,
    app: &mut App,
    menu: impl FnOnce(&mut Ui, &mut App),
) {
    // reserve the fill first so it can be painted under the text once the hover
    // state is known (egui's shape-index idiom)
    let fill = ui.painter().add(egui::Shape::Noop);
    let inner = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(12.0).monospace());
                ui.label(
                    RichText::new(icons::CHEVRON_DOWN)
                        .size(12.0)
                        .color(theme::FG_MUTED),
                );
            });
        })
        .response;
    // Vue: px-2 py-1
    let rect = inner.rect.expand2(egui::vec2(8.0, 4.0));
    let response = ui.interact(rect, egui::Id::new(id), egui::Sense::click());
    if response.hovered() {
        ui.painter().set(
            fill,
            egui::epaint::RectShape::filled(rect, 4.0, theme::SURFACE_HOVER),
        );
    }
    // Vue: bg-surface-elevated, border edge, shadow-lg, viewport py-1
    egui::Popup::menu(&response)
        .gap(4.0)
        .width(200.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_TERTIARY)
                .stroke(egui::Stroke::new(1.0, theme::EDGE))
                .corner_radius(0.0)
                .inner_margin(egui::Margin::symmetric(0, 4)),
        )
        .show(|ui| menu(ui, app));
}

/// One row of a select's menu: the Vue `SelectItem` — `px-3 py-2`, a hover fill
/// and a check on the value in use.
fn menu_item(ui: &mut Ui, label: &str, active: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
    if let Some(fill) = widgets::row_fill(false, response.hovered(), None) {
        ui.painter().rect_filled(rect, 0.0, fill);
    }
    ui.painter().text(
        egui::pos2(rect.left() + 12.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        if active { theme::ACCENT } else { theme::FG },
    );
    if active {
        ui.painter().text(
            egui::pos2(rect.right() - 12.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            icons::CHECK,
            egui::FontId::proportional(13.0),
            theme::ACCENT,
        );
    }
    response.clicked()
}
