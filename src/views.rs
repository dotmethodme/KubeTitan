//! Central content: the dashboard summary and the generic resource list view.
//!
//! Every resource type renders through [`resource_view`]; per-type differences
//! live in the catalog (`kinds.rs`), never here. That is what keeps 16 views
//! from becoming 16 copies of the same table code.

use eframe::egui::{self, RichText, Ui};

use crate::cluster;
use crate::kinds::{self, Kind, RowCtx};
use crate::widgets;
use crate::{App, View, theme};

/// Event table shared by the dashboard feed and the drawer's Events section.
/// `show_object` adds the dashboard's Object column, `show_badge` the drawer's
/// Type column — Vue renders the type as a badge in the drawer and as a dot on
/// the dashboard.
pub fn events_section(
    ui: &mut Ui,
    title: &str,
    events: &[cluster::EventRow],
    show_object: bool,
    show_badge: bool,
) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(10.5)
            .color(theme::FG_MUTED),
    );
    ui.add_space(4.0);
    if events.is_empty() {
        ui.label(RichText::new("No events").size(12.0).color(theme::FG_MUTED));
        return;
    }
    for ev in events {
        let width = ui.available_width().min(980.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 24.0), egui::Sense::hover());
        {
            let p = ui.painter();
            let msg_left = if show_badge {
                let (text, color) = if ev.warning {
                    ("Warning", theme::BAD)
                } else {
                    ("Normal", theme::FG_MUTED)
                };
                let badge = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 8.0, rect.center().y - 8.0),
                    egui::vec2(56.0, 16.0),
                );
                p.rect_filled(badge, 0.0, color.gamma_multiply(0.18));
                p.text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(10.5),
                    color,
                );
                let reason_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 74.0, rect.top()),
                    egui::pos2(rect.left() + 196.0, rect.bottom()),
                );
                p.with_clip_rect(reason_rect).text(
                    egui::pos2(reason_rect.left(), rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &ev.reason,
                    egui::FontId::monospace(11.5),
                    theme::FG,
                );
                rect.left() + 202.0
            } else {
                p.circle_filled(
                    egui::pos2(rect.left() + 8.0, rect.center().y),
                    3.5,
                    if ev.warning { theme::BAD } else { theme::OK },
                );
                let reason_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 20.0, rect.top()),
                    egui::pos2(rect.left() + 148.0, rect.bottom()),
                );
                p.with_clip_rect(reason_rect).text(
                    egui::pos2(reason_rect.left(), rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &ev.reason,
                    egui::FontId::monospace(12.0),
                    theme::FG,
                );
                if show_object {
                    let obj_rect = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + 152.0, rect.top()),
                        egui::pos2(rect.left() + 334.0, rect.bottom()),
                    );
                    p.with_clip_rect(obj_rect).text(
                        egui::pos2(obj_rect.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        format!("{}/{}", ev.object_kind, ev.object_name),
                        egui::FontId::proportional(12.0),
                        theme::FG_SECONDARY,
                    );
                }
                if show_object {
                    rect.left() + 340.0
                } else {
                    rect.left() + 152.0
                }
            };
            let msg_rect = egui::Rect::from_min_max(
                egui::pos2(msg_left, rect.top()),
                egui::pos2(rect.right() - 74.0, rect.bottom()),
            );
            p.with_clip_rect(msg_rect).text(
                egui::pos2(msg_rect.left(), rect.center().y),
                egui::Align2::LEFT_CENTER,
                &ev.message,
                egui::FontId::proportional(12.0),
                theme::FG_MUTED,
            );
            p.text(
                egui::pos2(rect.right() - 10.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                crate::kinds::human_age(ev.at),
                egui::FontId::monospace(11.0),
                theme::FG_MUTED,
            );
        }
        ui.add_space(2.0);
    }
}

fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::SURFACE)
        .inner_margin(egui::Margin::symmetric(16, 12))
}

/// Central area dispatch.
pub fn central(app: &mut App, ui: &mut Ui) {
    match app.view {
        View::Dashboard => dashboard(app, ui),
        View::PortForwards => port_forwards(app, ui),
        View::Resource(id) => match crate::kinds::by_id(id) {
            Some(kind) => resource_view(app, ui, kind),
            None => not_ported(ui, id),
        },
    }
}

/// The port-forward list's columns: the header and the rows share one shape, so
/// the columns cannot drift apart.
const FORWARD_COLUMNS: &[(&str, f32)] = &[
    ("Resource", 230.0),
    ("Type", 70.0),
    ("Namespace", 130.0),
    ("Remote", 80.0),
    ("Local", 170.0),
    ("Status", 90.0),
];

/// The Port Forwards view: what is forwarded, from where, to where. A forward is
/// a listener on this machine, so its local address is worth showing and copying.
pub fn port_forwards(app: &mut App, ui: &mut Ui) {
    let mut stopped: Option<String> = None;
    let mut started: Option<String> = None;
    let mut removed: Option<String> = None;
    let mut copied: Option<String> = None;
    panel_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Port Forwards").size(18.0).color(theme::FG));
            ui.label(
                RichText::new(format!(
                    "{} of {} connected",
                    app.forwards
                        .iter()
                        .filter(|f| f.status == crate::portforward::Status::Active)
                        .count(),
                    app.forwards.len()
                ))
                .size(12.0)
                .color(theme::FG_MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // The same secondary button the rest of the app uses, plus the
                // icon the Vue's "New Port Forward" carries. This was a raw
                // `egui::Button` filled with the accent, which kept egui's light
                // label colour: white on amber, and the only button in the app
                // that looked nothing like its neighbours.
                if widgets::secondary_button(ui, crate::icons::PLUS, "New port forward").clicked() {
                    app.forward_dialog = Some(crate::portforward::Dialog::for_resource(
                        &app.namespace,
                        crate::portforward::Target::Pod,
                        "",
                        "",
                        "",
                    ));
                }
            });
        });
        ui.add_space(12.0);

        if app.forwards.is_empty() {
            ui.label(
                RichText::new(
                    "No port forwards. Right-click a pod or a service and choose \
                     \u{201c}Forward port\u{2026}\u{201d}, or start one here.",
                )
                .size(12.5)
                .color(theme::FG_MUTED),
            );
            return;
        }

        // the columns, named once at the top of the list
        {
            let width = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::hover());
            let mut x = rect.left() + 26.0;
            for (title, size) in FORWARD_COLUMNS {
                ui.painter().text(
                    egui::pos2(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    *title,
                    egui::FontId::proportional(11.0),
                    theme::FG_MUTED,
                );
                x += size;
            }
            ui.painter().line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                egui::Stroke::new(1.0, theme::EDGE),
            );
        }

        for forward in &app.forwards {
            let width = ui.available_width();
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
            let status_color = match forward.status {
                crate::portforward::Status::Active => theme::OK,
                crate::portforward::Status::Stopped => theme::FG_MUTED,
                crate::portforward::Status::Error => theme::BAD,
            };
            ui.painter().circle_filled(
                egui::pos2(rect.left() + 10.0, rect.center().y),
                4.0,
                status_color,
            );

            // one shape for the header and the rows: the columns cannot drift
            let cells = [
                (forward.name.clone(), theme::FG),
                (forward.target.label().to_string(), theme::FG_SECONDARY),
                (forward.namespace.clone(), theme::FG_SECONDARY),
                (
                    // a service resolves to a pod port of its own: showing both
                    // is how the row says which number it actually opened
                    if forward.target_port == forward.remote_port {
                        forward.remote_port.to_string()
                    } else {
                        format!("{} \u{2192} {}", forward.remote_port, forward.target_port)
                    },
                    theme::FG_SECONDARY,
                ),
                (format!("127.0.0.1:{}", forward.local_port), theme::FG),
                (forward.status.label().to_string(), status_color),
            ];
            let mut x = rect.left() + 26.0;
            for ((_, column), (text, color)) in FORWARD_COLUMNS.iter().zip(cells) {
                let cell = egui::Rect::from_min_size(
                    egui::pos2(x, rect.top()),
                    egui::vec2(*column, rect.height()),
                );
                // a long name must not run into the column beside it
                ui.painter().with_clip_rect(cell).text(
                    egui::pos2(cell.left(), cell.center().y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    egui::FontId::monospace(11.5),
                    color,
                );
                x += column;
            }

            if forward.status == crate::portforward::Status::Error
                && let Some(error) = &forward.error
            {
                response.clone().on_hover_text(error);
            }
            let id = forward.id.clone();
            let name = forward.name.clone();
            let local = format!("127.0.0.1:{}", forward.local_port);
            let connected = forward.status == crate::portforward::Status::Active;
            response.context_menu(|ui| {
                ui.set_min_width(200.0);
                if ui.button("Copy local address").clicked() {
                    copied = Some(local.clone());
                    ui.close();
                }
                if connected {
                    if ui.button("Disconnect").clicked() {
                        stopped = Some(id.clone());
                        ui.close();
                    }
                } else if ui.button("Connect").clicked() {
                    started = Some(id.clone());
                    ui.close();
                }
                ui.separator();
                if ui.button("Remove from saved").clicked() {
                    removed = Some(id.clone());
                    ui.close();
                }
            });
            response.on_hover_text(format!("{}/{}", forward.namespace, name));
        }
    });

    if let Some(text) = copied {
        ui.ctx().copy_text(text);
        app.copied_at = Some(std::time::Instant::now());
    }
    if let Some(id) = stopped {
        match crate::portforward::stop(app, &id) {
            Ok(()) => app.flash_status("Port forward disconnected".to_string()),
            Err(error) => app.flash_error(error),
        }
    }
    if let Some(id) = started {
        match crate::portforward::connect(app, &id) {
            Ok(()) => app.flash_status("Port forward connected".to_string()),
            Err(error) => app.flash_error(error),
        }
    }
    if let Some(id) = removed {
        match crate::portforward::remove(app, &id) {
            Ok(()) => app.flash_status("Port forward removed".to_string()),
            Err(error) => app.flash_error(error),
        }
    }
}

fn not_ported(ui: &mut Ui, what: &str) {
    egui::CentralPanel::default()
        .frame(panel_frame())
        .show(ui, |ui| {
            let width = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 220.0), egui::Sense::hover());
            ui.painter().text(
                egui::pos2(rect.center().x, rect.top() + 70.0),
                egui::Align2::CENTER_CENTER,
                crate::icons::HOURGLASS,
                egui::FontId::proportional(32.0),
                theme::FG_MUTED,
            );
            ui.painter().text(
                egui::pos2(rect.center().x, rect.top() + 106.0),
                egui::Align2::CENTER_CENTER,
                format!("{what} is not ported yet"),
                egui::FontId::proportional(13.0),
                theme::FG_MUTED,
            );
        });
}

// ---------------------------------------------------------------------------
// dashboard
// ---------------------------------------------------------------------------

fn dashboard(app: &mut App, ui: &mut Ui) {
    let counts = app.workload_counts();
    egui::CentralPanel::default()
        .frame(panel_frame())
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("dashboard_scroll")
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Dashboard").size(16.0).strong());
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!(
                            "context {} \u{2022} {} namespaces in view",
                            app.context_label(),
                            app.namespaces.len()
                        ))
                        .size(12.0)
                        .color(theme::FG_MUTED),
                    );
                    ui.add_space(14.0);
                    for (label, value, icon) in counts {
                        let width = ui.available_width().min(460.0);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(width, 44.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 0.0, theme::SURFACE_SECONDARY);
                        ui.painter().text(
                            egui::pos2(rect.left() + 14.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            icon,
                            egui::FontId::proportional(16.0),
                            theme::FG_MUTED,
                        );
                        ui.painter().text(
                            egui::pos2(rect.left() + 44.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(13.0),
                            theme::FG_SECONDARY,
                        );
                        ui.painter().text(
                            egui::pos2(rect.right() - 14.0, rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            value,
                            egui::FontId::monospace(13.0),
                            theme::FG,
                        );
                        ui.add_space(6.0);
                    }

                    // Recent Events: the dashboard's "what just happened" feed.
                    ui.separator();
                    events_section(ui, "Recent Events", &app.events, true, false);
                    apps_section(app, ui);
                });
        });
}

/// The dashboard's applications grid: deployments and statefulsets as cards,
/// health first — the visual overview the Vue dashboard closes with.
fn apps_section(app: &mut App, ui: &mut Ui) {
    const CARD_H: f32 = 68.0;
    const MIN_W: f32 = 220.0;
    const GAP: f32 = 12.0;

    ui.separator();
    ui.add_space(8.0);
    ui.label(
        RichText::new(format!("{}  APPLICATIONS", crate::icons::APPS))
            .size(10.5)
            .color(theme::FG_MUTED),
    );
    ui.add_space(8.0);

    if app.apps.is_empty() {
        widgets::empty_state(
            ui,
            crate::icons::CUBE_OFF_OUTLINE,
            "No deployments or statefulsets in this namespace",
        );
        return;
    }

    let avail = ui.available_width();
    let cols = (((avail + GAP) / (MIN_W + GAP)).floor() as usize).max(1);
    let card_w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();

    let mut open: Option<&'static str> = None;
    for chunk in app.apps.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for row in chunk {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(card_w, CARD_H), egui::Sense::click());
                app_card(ui, rect, row, response.hovered());
                if response.clicked() {
                    open = Some(row.view_id());
                }
            }
        });
        ui.add_space(GAP);
    }

    // a card jumps to the list view for its kind, like the Vue grid's click
    if let Some(kind) = open.and_then(kinds::by_id) {
        app.set_view(View::Resource(kind.id));
    }
}

/// One application card: a health dot, the name, its kind and ready/desired.
fn app_card(ui: &Ui, rect: egui::Rect, row: &cluster::AppRow, hovered: bool) {
    let (fill, edge) = if hovered {
        (theme::SURFACE_HOVER, theme::EDGE_HOVER)
    } else {
        (theme::SURFACE_SECONDARY, theme::EDGE)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, fill);
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, edge),
        egui::StrokeKind::Inside,
    );

    let color = match row.status() {
        cluster::AppStatus::Healthy => theme::OK,
        cluster::AppStatus::Degraded => theme::WARN,
        cluster::AppStatus::Unavailable => theme::BAD,
    };
    // Vue draws an 8px square with a coloured box-shadow; fading rings stand in
    let dot = egui::pos2(rect.left() + 20.0, rect.center().y);
    for (size, alpha) in [(22.0, 0.08), (15.0, 0.16)] {
        painter.rect_filled(
            egui::Rect::from_center_size(dot, egui::vec2(size, size)),
            2.0,
            color.gamma_multiply(alpha),
        );
    }
    painter.rect_filled(
        egui::Rect::from_center_size(dot, egui::vec2(8.0, 8.0)),
        0.0,
        color,
    );

    let left = dot.x + 14.0;
    let right = rect.right() - 16.0;
    // a long name ellipsises instead of spilling into the next card
    let painter = painter.with_clip_rect(egui::Rect::from_min_max(
        egui::pos2(left, rect.top()),
        egui::pos2(right, rect.bottom()),
    ));
    painter.text(
        egui::pos2(left, rect.top() + 24.0),
        egui::Align2::LEFT_CENTER,
        &row.name,
        egui::FontId::monospace(13.0),
        theme::FG,
    );
    painter.text(
        egui::pos2(left, rect.bottom() - 22.0),
        egui::Align2::LEFT_CENTER,
        capitalised(row.kind),
        egui::FontId::proportional(12.0),
        theme::FG_MUTED,
    );
    painter.text(
        egui::pos2(right, rect.bottom() - 22.0),
        egui::Align2::RIGHT_CENTER,
        format!("{}/{}", row.ready, row.desired),
        egui::FontId::monospace(12.0),
        theme::FG_MUTED,
    );
}

/// Vue's `capitalize` class: first letter up, the rest untouched.
fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// generic resource view
// ---------------------------------------------------------------------------

fn resource_view(app: &mut App, ui: &mut Ui, kind: &'static Kind) {
    // The order, not the rows: the table asks for the cells of the rows it is
    // about to paint, so only those get extracted (see `widgets::table`).
    let objects = app.ordered(kind);
    let metrics = app.metrics();
    let ctx = kinds::RowCtx { metrics: &metrics };
    let total = app.unfiltered_count(kind);
    let filtered = objects.len();
    // the search box edits this copy and it is written back after the panel: the
    // panel's closure holds the view's objects borrowed, so it cannot also take
    // the app mutably. Same one-frame lag the field always had.
    let mut query = app.query.clone();
    // read before the panel for the same reason, and cleared after it
    let focus_search = app.focus_search;
    let mut clicked = None;
    let mut action = None;
    let mut refresh = false;

    egui::CentralPanel::default()
        .frame(panel_frame())
        .show(ui, |ui| {
            let subtitle = if filtered < total {
                format!(
                    "{filtered} of {total} {} in {}",
                    kinds::count_label(kind),
                    app.namespace_label()
                )
            } else {
                format!(
                    "{total} {} in {}",
                    kinds::count_label(kind),
                    app.namespace_label()
                )
            };
            let query_label = app.query.clone();
            refresh = widgets::page_header(
                ui,
                kind.label,
                &subtitle,
                &mut query,
                filtered,
                total,
                focus_search,
            );
            ui.add_space(6.0);

            if app.loading {
                widgets::skeleton(ui);
                return;
            }

            if objects.is_empty() {
                let (icon, text) = if query_label.is_empty() {
                    (
                        crate::icons::CUBE_OFF_OUTLINE,
                        format!("No {} found in this namespace", kinds::count_label(kind)),
                    )
                } else {
                    (
                        crate::icons::TEXT_SEARCH,
                        format!("No {} match your search", kinds::count_label(kind)),
                    )
                };
                widgets::empty_state(ui, icon, &text);
                return;
            }

            let selected = app.selected_key();
            let events = widgets::table(
                ui,
                kind,
                objects.len(),
                |index| row_for(kind, objects[index], &ctx),
                selected.as_deref(),
            );
            clicked = events.clicked;
            action = events.action;
        });

    app.query = query;
    app.focus_search = false;
    if refresh {
        app.refresh_view();
    }
    if let Some(key) = clicked {
        app.select(kind, &key);
    }
    if let Some((action, key)) = action {
        app.begin_action(kind, &key, action);
    }
}

/// Build one table row from a cluster object.
pub fn row_for(kind: &Kind, obj: &kube::core::DynamicObject, ctx: &RowCtx) -> widgets::RowData {
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();
    let name = obj.metadata.name.clone().unwrap_or_default();
    let mut cells = (kind.cells)(obj, ctx);
    // never let a short extractor shift the columns
    while cells.len() < kind.columns.len() {
        cells.push(crate::kinds::Cell::empty());
    }
    widgets::RowData {
        key: format!("{namespace}/{name}"),
        namespace,
        name,
        cells,
    }
}
