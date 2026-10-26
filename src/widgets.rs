//! Reusable UI primitives.
//!
//! These deliberately paint cells at absolute coordinates instead of nesting
//! layouts: egui's nested layout engine is where table alignment goes wrong,
//! and the Vue tables being ported have exact column widths and paddings.

use eframe::egui::{self, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui, Vec2};

use crate::kinds::{Action, Cell, Column, Kind, Width};
use crate::theme;

/// Vue: `table th { padding: 12px 16px }`, rows 44px tall.
pub const ROW_H: f32 = 32.0;
const CELL_PAD: f32 = 8.0;
const DOT_RADIUS: f32 = 3.5;

// ---------------------------------------------------------------------------
// page header
// ---------------------------------------------------------------------------

/// Title + count subtitle + Refresh button, mirroring the Vue `.page-header`.
/// Returns true when Refresh was pressed.
pub fn page_header(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    query: &mut String,
    filtered: usize,
    total: usize,
    focus_search: bool,
) -> bool {
    let mut refresh = false;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 56.0), Sense::hover());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new(title).size(16.0).strong().color(theme::FG));
            let sub = if !query.is_empty() {
                format!("{filtered} of {total} match \u{201c}{query}\u{201d}")
            } else {
                subtitle.to_string()
            };
            ui.label(RichText::new(sub).size(12.0).color(theme::FG_MUTED));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, crate::icons::REFRESH, "Refresh").clicked() {
                refresh = true;
            }
            ui.add_space(8.0);
            let next = search_field(ui, query, focus_search);
            *query = next;
        });
    });
    refresh
}

/// Filled line chart of `(timestamp, value)` samples — the drawer's usage
/// timelines. Returns the rect it painted, so callers can add axis labels.
pub fn sparkline(ui: &mut Ui, points: &[(i64, f64)], size: Vec2) -> Rect {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, theme::SURFACE_TERTIARY);
    if points.len() < 2 {
        return rect;
    }

    let (first, last) = (points[0].0, points[points.len() - 1].0);
    let span = (last - first).max(1) as f32;
    let peak = points
        .iter()
        .map(|(_, value)| *value)
        .fold(0.0_f64, f64::max);
    let peak = if peak <= 0.0 { 1.0 } else { peak * 1.15 };

    let inner = rect.shrink2(Vec2::new(4.0, 4.0));
    let line: Vec<egui::Pos2> = points
        .iter()
        .map(|(timestamp, value)| {
            let x = inner.left() + ((timestamp - first) as f32 / span) * inner.width();
            let y = inner.bottom() - ((value / peak) as f32).clamp(0.0, 1.0) * inner.height();
            egui::pos2(x, y)
        })
        .collect();

    let mut area = line.clone();
    area.push(egui::pos2(inner.right(), inner.bottom()));
    area.push(egui::pos2(inner.left(), inner.bottom()));
    painter.add(egui::Shape::Path(egui::epaint::PathShape {
        points: area,
        closed: true,
        fill: theme::ACCENT.gamma_multiply(0.18),
        stroke: egui::epaint::PathStroke::NONE,
    }));
    painter.add(egui::Shape::line(line, Stroke::new(1.5, theme::ACCENT)));
    rect
}

/// `0.0123` cores → `12m`, the same shape as the pod table's CPU column.
pub fn fmt_cores(cores: f64) -> String {
    let milli = (cores * 1000.0).round();
    if milli < 1000.0 {
        format!("{milli:.0}m")
    } else {
        format!("{cores:.1}")
    }
}

/// Bytes → `Ki`/`Mi`/`Gi`, as in the pod table's Memory column.
pub fn fmt_bytes(bytes: f64) -> String {
    const UNITS: [&str; 4] = ["Ki", "Mi", "Gi", "Ti"];
    let mut value = bytes / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 10.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// Unified diff of two YAML documents, line by line — the Vue app previews the
/// same diff before an apply. `similar` supplies the algorithm so the diff is
/// not hand-rolled.
pub fn yaml_diff(ui: &mut Ui, before: &str, after: &str) {
    let diff = similar::TextDiff::from_lines(before, after);
    let text = diff
        .unified_diff()
        .context_radius(3)
        .header("current", "edited")
        .to_string();

    for line in text.lines() {
        let color = if line.starts_with("+++") || line.starts_with("---") {
            theme::FG_MUTED
        } else if line.starts_with('+') {
            theme::OK
        } else if line.starts_with('-') {
            theme::BAD
        } else if line.starts_with("@@") {
            theme::ACCENT
        } else {
            theme::FG_SECONDARY
        };
        ui.label(RichText::new(line).monospace().size(11.5).color(color));
    }
}

/// Renders the keyboard-shortcuts list (Vue's `KeyboardShortcutsDialog`,
/// toggled with Ctrl+/).
///
/// Only shortcuts this port actually implements are listed — the dialog is a
/// contract with the user, so it must not advertise keys that do nothing.
pub fn shortcuts_dialog(ctx: &egui::Context, open: &mut bool) {
    if !*open {
        return;
    }
    const GROUPS: &[(&str, &[(&str, &str)])] = &[
        (
            "Navigation",
            &[
                ("Ctrl P", "Open command palette"),
                ("Ctrl F", "Search the current view"),
                ("Ctrl .", "Switch namespace"),
                ("Ctrl ,", "Switch context"),
                ("Ctrl /", "Show keyboard shortcuts"),
                ("Esc", "Close palette or dialog"),
            ],
        ),
        (
            "Quick Switch",
            &[
                ("Alt 1-9", "Switch to context 1-9"),
                ("Ctrl 1-9", "Switch to view 1-9, sidebar order"),
            ],
        ),
        (
            "Command Palette",
            &[
                ("\u{2191} \u{2193}", "Navigate through options"),
                ("Enter", "Select option"),
                ("ns:", "Filter namespaces (type in the palette)"),
                ("ctx:", "Filter contexts (type in the palette)"),
            ],
        ),
    ];

    egui::Modal::new(egui::Id::new("shortcuts-dialog"))
        .backdrop_color(Color32::from_black_alpha(140))
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_SECONDARY)
                .stroke(Stroke::new(1.0, theme::EDGE))
                .corner_radius(0.0)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.set_width(430.0);
            ui.label(
                RichText::new("Keyboard Shortcuts")
                    .size(15.0)
                    .strong()
                    .color(theme::FG),
            );
            ui.add_space(12.0);
            for (group, rows) in GROUPS {
                ui.label(
                    RichText::new(group.to_uppercase())
                        .size(10.5)
                        .color(theme::FG_MUTED),
                );
                ui.add_space(5.0);
                for (keys, description) in *rows {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(84.0, 20.0), Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, theme::SURFACE_TERTIARY);
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            *keys,
                            FontId::monospace(11.5),
                            theme::FG,
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(*description)
                                .size(12.5)
                                .color(theme::FG_SECONDARY),
                        );
                    });
                    ui.add_space(3.0);
                }
                ui.add_space(10.0);
            }
            ui.separator();
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary_button(ui, '\0', "Close").clicked() {
                    *open = false;
                }
            });
        });
}

/// Rounded search box with the mdi magnify glyph, matching the app's inputs.
/// Returns the edited value; the caller stores it.
/// Vue's search input. `focus` lets a shortcut (Ctrl+F) put the caret in it.
pub fn search_field(ui: &mut Ui, query: &str, focus: bool) -> String {
    let mut out = query.to_string();
    let height = 30.0;
    let width = 260.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::SURFACE_TERTIARY);
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, theme::EDGE),
        egui::StrokeKind::Inside,
    );
    let icon_rect = Rect::from_min_size(rect.min + Vec2::new(8.0, 0.0), Vec2::new(18.0, height));
    ui.painter().text(
        icon_rect.center(),
        Align2::CENTER_CENTER,
        crate::icons::MAGNIFY,
        FontId::proportional(14.0),
        theme::FG_MUTED,
    );
    let field_rect = Rect::from_min_max(
        rect.min + Vec2::new(28.0, 4.0),
        rect.max - Vec2::new(6.0, 4.0),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field_rect));
    let resp = child.add(
        egui::TextEdit::singleline(&mut out)
            .hint_text("Search…")
            .desired_width(field_rect.width())
            .background_color(egui::Color32::TRANSPARENT)
            .vertical_align(egui::Align::Center),
    );
    if focus {
        resp.request_focus();
    }
    out
}

/// Vue `.btn-secondary`: surface-secondary fill, 1px edge, 6px radius.
/// Shared painter for the filled dialog buttons: accent for the primary
/// action, danger for the destructive one.
fn filled_button(ui: &mut Ui, label: &str, fill: Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        FontId::proportional(12.0),
        theme::SURFACE,
    );
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(galley.size().x + 20.0, 24.0), Sense::click());
    let fill = if response.hovered() {
        theme::alpha(fill, 0.85)
    } else {
        fill
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, theme::SURFACE);
    response
}

/// Accent-filled button — a view's or dialog's primary action.
pub fn primary_button(ui: &mut Ui, label: &str) -> egui::Response {
    filled_button(ui, label, theme::ACCENT)
}

/// Danger-filled button — a destructive confirmation.
pub fn danger_button(ui: &mut Ui, label: &str) -> egui::Response {
    filled_button(ui, label, theme::BAD)
}

pub fn secondary_button(ui: &mut Ui, icon: char, label: &str) -> egui::Response {
    button(
        ui,
        icon,
        label,
        theme::SURFACE_SECONDARY,
        theme::FG_SECONDARY,
        1.0,
    )
}

/// Shared button painter: optional icon, label, hover state, 1px edge.
fn button(
    ui: &mut Ui,
    icon: char,
    label: &str,
    fill: Color32,
    fg: Color32,
    stroke_w: f32,
) -> egui::Response {
    let text = if icon == '\0' {
        label.to_string()
    } else if label.is_empty() {
        icon.to_string()
    } else {
        format!("{icon}  {label}")
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text, FontId::proportional(13.0), fg);
    let size = Vec2::new(galley.size().x + 24.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        theme::SURFACE_HOVER
    } else {
        fill
    };
    ui.painter().rect_filled(rect, 0.0, fill);
    if stroke_w > 0.0 {
        ui.painter().rect_stroke(
            rect,
            0.0,
            Stroke::new(
                stroke_w,
                if resp.hovered() {
                    theme::EDGE_HOVER
                } else {
                    theme::EDGE
                },
            ),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

/// A checkbox: square, small, with the app's own check glyph.
///
/// egui's built-in one is a rounded box with a polyline tick, which is neither
/// the shape nor the mark the Vue's native checkboxes (`accent-accent`) show.
/// The whole label toggles, as a `<label>` wrapping an input does.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, label: &str) -> egui::Response {
    const BOX: f32 = 13.0;
    const GAP: f32 = 6.0;
    let font = FontId::proportional(11.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), theme::FG_MUTED);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(BOX + GAP + galley.size().x, BOX.max(galley.size().y)),
        Sense::click(),
    );
    if response.clicked() {
        *checked = !*checked;
    }

    let box_rect = Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - BOX / 2.0),
        Vec2::splat(BOX),
    );
    let hovered = response.hovered();
    let edge = match (*checked, hovered) {
        (true, _) => theme::ACCENT,
        (false, true) => theme::EDGE_HOVER,
        (false, false) => theme::EDGE,
    };
    ui.painter().rect_filled(
        box_rect,
        0.0,
        if *checked {
            theme::ACCENT
        } else {
            theme::SURFACE
        },
    );
    ui.painter().rect_stroke(
        box_rect,
        0.0,
        egui::Stroke::new(1.0, edge),
        egui::StrokeKind::Inside,
    );
    if *checked {
        ui.painter().text(
            box_rect.center(),
            Align2::CENTER_CENTER,
            crate::icons::CHECK,
            FontId::proportional(11.0),
            theme::SURFACE,
        );
    }
    ui.painter().text(
        egui::pos2(box_rect.right() + GAP, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        theme::FG_MUTED,
    );
    response
}

/// A select: square, sized to its text, with the app's chevron for an arrow.
///
/// The Vue's `.select` carries no radius and sits at `width: auto`, and its
/// arrow is a stroke chevron; egui's combo draws a filled triangle in a box the
/// height of the app's largest control, which is what made a two-container pod
/// look like it had a piece of another UI in its toolbar.
pub fn select(ui: &mut Ui, current: &str, options: &[String]) -> Option<String> {
    const HEIGHT: f32 = 22.0;
    let font = FontId::proportional(11.5);
    let galley = ui
        .painter()
        .layout_no_wrap(current.to_owned(), font.clone(), theme::FG);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new((galley.size().x + 34.0).max(72.0), HEIGHT),
        Sense::click(),
    );
    let hovered = response.hovered();
    ui.painter()
        .rect_filled(rect, 0.0, theme::SURFACE_SECONDARY);
    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(
            1.0,
            if hovered {
                theme::EDGE_HOVER
            } else {
                theme::EDGE
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        current,
        font,
        theme::FG,
    );
    ui.painter().text(
        egui::pos2(rect.right() - 10.0, rect.center().y),
        Align2::CENTER_CENTER,
        crate::icons::CHEVRON_DOWN,
        FontId::proportional(12.0),
        theme::FG_MUTED,
    );

    let mut chosen = None;
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(rect.width());
        for option in options {
            if ui.selectable_label(option == current, option).clicked() {
                chosen = Some(option.clone());
                ui.close();
            }
        }
    });
    chosen
}

/// Small icon-only button (drawer close, etc).
pub fn icon_button(ui: &mut Ui, icon: char, color: Color32, hover: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::SURFACE_HOVER);
    }
    // the glyph itself changes colour on hover, as the Vue's ghost buttons do
    let tint = if resp.hovered() { hover } else { color };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(16.0),
        tint,
    );
    resp
}

// ---------------------------------------------------------------------------
// table
// ---------------------------------------------------------------------------

/// One table row: identity plus the pre-extracted cells.
pub struct RowData {
    /// stable key (`namespace/name`), used for selection and actions
    pub key: String,
    pub namespace: String,
    pub name: String,
    pub cells: Vec<Cell>,
}

/// What the user did to the table this frame.
#[derive(Default)]
pub struct TableEvents {
    /// A row was clicked (select + open drawer).
    pub clicked: Option<String>,
    /// A context-menu action was chosen.
    pub action: Option<(Action, String)>,
    pub hovered: Option<String>,
}

/// Allocate a full-width row that senses hover and clicks — the shape every
/// list row in the app has.
pub fn row(ui: &mut Ui, height: f32) -> (egui::Rect, egui::Response) {
    ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click())
}

/// A row that can also be picked up. Only the sidebar's favorites use it: a
/// list the user built is a list they get to order, while every other row stays
/// click-only so a stray drag cannot start on it.
pub fn row_draggable(ui: &mut Ui, height: f32) -> (egui::Rect, egui::Response) {
    ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::click_and_drag(),
    )
}

/// Background fill for a list row. The selected row wins over the hovered one,
/// and a resting row falls back to `rest` — a table stripe, or nothing at all.
///
/// The palette and the resource picker deliberately don't use this: the palette
/// highlights its cursor the same way it highlights hover, and a picker row
/// sits on a dialog surface, so its hover is a different token.
pub fn row_fill(
    selected: bool,
    hovered: bool,
    rest: Option<egui::Color32>,
) -> Option<egui::Color32> {
    if selected {
        Some(theme::ACCENT_MUTED)
    } else if hovered {
        Some(theme::SURFACE_HOVER)
    } else {
        rest
    }
}

pub fn table(
    ui: &mut Ui,
    kind: &Kind,
    row_count: usize,
    row: impl Fn(usize) -> RowData,
    selected: Option<&str>,
) -> TableEvents {
    let mut events = TableEvents::default();
    // The column labels sit outside the scroll area so they stay put while the
    // rows under them move: a header that scrolls away leaves every column
    // unlabelled, which is worst exactly when the list is long.
    paint_header(ui, kind, ui.available_width());
    // A uniform row height is what lets the scroll area paint only the rows on
    // screen. Laying out the whole list every frame is what makes a long table
    // feel slow: 135 pods is 135 rows of cells for one visible screenful.
    egui::ScrollArea::vertical()
        .id_salt(("table", kind.id))
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, row_count, |ui, range| {
            let width = ui.available_width();
            let widths = column_widths(kind.columns, width);

            for index in range {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());

                // the row the table is about to paint, and only now does its
                // cells get extracted
                let data = row(index);
                let row = &data;
                let row_rect = rect;
                let is_selected = selected == Some(row.key.as_str());
                // Vue's `table tbody tr:nth-child(even)` striping: with the
                // header out of the list, the first data row is the odd one
                let stripe = (index % 2 == 1).then_some(theme::STRIPE);
                if let Some(fill) = row_fill(is_selected, resp.hovered(), stripe) {
                    ui.painter().rect_filled(row_rect, 0.0, fill);
                }
                if resp.hovered() {
                    events.hovered = Some(row.key.clone());
                }

                let mut x = row_rect.left();
                for (idx, cell) in row.cells.iter().enumerate() {
                    let w = *widths.get(idx).unwrap_or(&120.0);
                    let cell_rect =
                        Rect::from_min_size(egui::pos2(x, row_rect.top()), Vec2::new(w, ROW_H));
                    paint_cell(
                        ui,
                        cell_rect,
                        cell,
                        kind.columns.get(idx).is_none_or(|c| c.numeric),
                        is_selected,
                    );
                    x += w;
                }

                if resp.clicked() {
                    events.clicked = Some(row.key.clone());
                }

                // right-click menu: the Vue views' ContextMenuContent items
                if !kind.actions.is_empty() {
                    let key = row.key.clone();
                    let actions = kind.actions;
                    let mut chosen = None;
                    resp.context_menu(|ui| {
                        ui.set_min_width(200.0);
                        for action in actions {
                            let label = match action {
                                Action::Delete => format!("Delete {}", kind.kind),
                                other => other.label().to_string(),
                            };
                            let color = if action.destructive() {
                                theme::BAD
                            } else {
                                theme::FG_SECONDARY
                            };
                            let text = format!("{}   {label}", action.icon());
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(text).color(color))
                                        .frame(false),
                                )
                                .clicked()
                            {
                                chosen = Some(*action);
                                ui.close();
                            }
                        }
                    });
                    if let Some(action) = chosen {
                        events.action = Some((action, key));
                    }
                }

                // hovering a clipped name column still reveals the full identity
                resp.on_hover_text(format!("{}/{}", row.namespace, row.name));
            }
        });

    events
}

/// The table's column labels, painted once above the scrolling rows.
fn paint_header(ui: &mut Ui, kind: &Kind, width: f32) {
    let widths = column_widths(kind.columns, width);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::hover());
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::EDGE),
    );
    let mut x = rect.left();
    for (idx, col) in kind.columns.iter().enumerate() {
        let w = widths[idx];
        let cell = Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(w, ROW_H));
        paint_cell_text(
            ui,
            cell,
            &Cell {
                text: col.title.to_owned(),
                color: Some(theme::FG_MUTED),
                mono: false,
                ..Default::default()
            },
            11.0,
            col.numeric,
        );
        x += w;
    }
}

pub fn column_widths(columns: &[Column], total: f32) -> Vec<f32> {
    let fixed: f32 = columns
        .iter()
        .filter_map(|c| match c.width {
            Width::Fixed(w) => Some(w),
            Width::Fill => None,
        })
        .sum();
    let fill_count = columns
        .iter()
        .filter(|c| matches!(c.width, Width::Fill))
        .count()
        .max(1);
    let remainder = ((total - fixed) / fill_count as f32).max(140.0);
    columns
        .iter()
        .map(|c| match c.width {
            Width::Fixed(w) => w,
            Width::Fill => remainder,
        })
        .collect()
}

fn paint_cell(ui: &mut Ui, rect: Rect, cell: &Cell, numeric: bool, selected: bool) {
    let clip = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let mut content = rect;
    if let Some(color) = cell.dot {
        let c = egui::pos2(rect.left() + CELL_PAD + DOT_RADIUS, rect.center().y);
        clip.circle_filled(c, DOT_RADIUS, color);
        content.min.x += CELL_PAD + DOT_RADIUS * 2.0 + 8.0;
    } else {
        content.min.x += CELL_PAD;
    }
    content.max.x -= CELL_PAD;

    if let Some(badge) = cell.badge {
        let galley = clip.layout_no_wrap(cell.text.clone(), FontId::proportional(11.0), badge);
        let size = Vec2::new(galley.size().x + 16.0, 20.0);
        let badge_rect = Rect::from_center_size(
            egui::pos2(content.left() + size.x / 2.0, rect.center().y),
            size,
        );
        clip.rect_filled(badge_rect, 0.0, theme::alpha(badge, 0.16));
        clip.galley(badge_rect.center() - galley.size() / 2.0, galley, badge);
        return;
    }

    let color = cell.color.unwrap_or(if selected {
        theme::FG
    } else {
        theme::FG_SECONDARY
    });
    paint_cell_text(ui, content, cell, 13.0, numeric);
    let _ = color;
}

/// Paints cell text inside `rect`, left- or right-aligned, clipped to the cell.
fn paint_cell_text(ui: &mut Ui, rect: Rect, cell: &Cell, size: f32, numeric: bool) {
    if cell.text.is_empty() {
        return;
    }
    let clip = ui.painter().with_clip_rect(rect);
    let font = if cell.mono {
        FontId::monospace(size)
    } else {
        FontId::proportional(size)
    };
    let color = cell.color.unwrap_or(theme::FG_SECONDARY);
    let galley = clip.layout_no_wrap(cell.text.clone(), font, color);

    // horizontal padding only applies to text columns; the dot already offsets
    let mut content = rect;
    if cell.dot.is_none() && cell.badge.is_none() {
        content.min.x += CELL_PAD;
        content.max.x -= CELL_PAD;
    }
    if numeric {
        let pos = egui::pos2(
            content.right() - galley.size().x,
            rect.center().y - galley.size().y / 2.0,
        );
        clip.galley(pos, galley, color);
    } else {
        let pos = egui::pos2(content.left(), rect.center().y - galley.size().y / 2.0);
        clip.galley(pos, galley, color);
    }
}

// ---------------------------------------------------------------------------
// states
// ---------------------------------------------------------------------------

pub fn empty_state(ui: &mut Ui, icon: char, text: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 220.0), Sense::hover());
    ui.painter().text(
        egui::pos2(rect.center().x, rect.top() + 70.0),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(34.0),
        theme::FG_MUTED,
    );
    ui.painter().text(
        egui::pos2(rect.center().x, rect.top() + 108.0),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(13.0),
        theme::FG_MUTED,
    );
}

/// Vue shows three skeleton rows while a view loads.
pub fn skeleton(ui: &mut Ui) {
    for _ in 0..3 {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::hover());
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(width, 40.0)),
            0.0,
            theme::SURFACE_TERTIARY,
        );
        ui.add_space(8.0);
    }
}

// ---------------------------------------------------------------------------
// tabs
// ---------------------------------------------------------------------------

/// Underlined tab strip (Vue `.tabs .tab.active`). Returns the clicked index.
pub fn tabs(ui: &mut Ui, labels: &[(String, Option<String>)], active: usize) -> Option<usize> {
    let mut clicked = None;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::hover());
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::EDGE),
    );
    // the strip lines up with the header's title and the content's own padding
    let mut x = rect.left() + 16.0;
    for (idx, (label, count)) in labels.iter().enumerate() {
        let text = match count {
            Some(c) => format!("{label}  {c}"),
            None => label.clone(),
        };
        let is_active = idx == active;
        let color = if is_active {
            theme::FG
        } else {
            theme::FG_MUTED
        };
        let galley = ui
            .painter()
            .layout_no_wrap(text, FontId::proportional(13.0), color);
        let w = galley.size().x + 24.0;
        let tab_rect = Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(w, rect.height()));
        let resp = ui.interact(tab_rect, ui.id().with(("tab", idx)), Sense::click());
        if resp.hovered() && !is_active {
            ui.painter()
                .rect_filled(tab_rect, 0.0, theme::SURFACE_HOVER);
        }
        ui.painter().galley(
            egui::pos2(
                tab_rect.center().x - galley.size().x / 2.0,
                tab_rect.center().y - galley.size().y / 2.0 - 1.0,
            ),
            galley,
            color,
        );
        if is_active {
            ui.painter().line_segment(
                [
                    egui::pos2(tab_rect.left(), rect.bottom() - 1.0),
                    egui::pos2(tab_rect.right(), rect.bottom() - 1.0),
                ],
                Stroke::new(2.0, theme::ACCENT),
            );
        }
        if resp.clicked() {
            clicked = Some(idx);
        }
        x += w;
    }
    clicked
}

// ---------------------------------------------------------------------------
// dialogs
// ---------------------------------------------------------------------------

pub enum Confirm {
    Cancel,
    Accept,
    /// Dialog stays open.
    None,
}

/// What the user chose in the YAML apply dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum YamlChoice {
    Cancel,
    /// `force_conflicts`: Vue's "Force Apply" (`true`) vs "Apply Changes".
    Apply(bool),
    /// Dialog stays open.
    None,
}

/// Vue's `ResourceYamlApplyDialog`: the diff of what will be applied, then a
/// server-side apply. Errors are shown in place so the draft can be fixed.
pub fn yaml_apply_dialog(
    ctx: &egui::Context,
    open: bool,
    before: &str,
    after: &str,
    applying: bool,
    error: Option<&str>,
) -> YamlChoice {
    if !open {
        return YamlChoice::None;
    }
    let mut result = YamlChoice::None;
    egui::Modal::new(egui::Id::new("yaml-apply-dialog"))
        .backdrop_color(Color32::from_black_alpha(140))
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_SECONDARY)
                .stroke(Stroke::new(1.0, theme::EDGE))
                .corner_radius(0.0)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.set_width(660.0);
            ui.label(
                RichText::new("Apply YAML Changes")
                    .size(15.0)
                    .strong()
                    .color(theme::FG),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("Review the diff before running a server-side apply.")
                    .size(12.0)
                    .color(theme::FG_MUTED),
            );
            ui.add_space(10.0);
            egui::Frame::new()
                .fill(theme::SURFACE)
                .corner_radius(0.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    egui::ScrollArea::both()
                        .id_salt("yaml-apply-diff")
                        .max_height(360.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| yaml_diff(ui, before, after));
                });

            if let Some(error) = error {
                ui.add_space(10.0);
                ui.label(RichText::new(error).size(12.0).color(theme::BAD));
            }

            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if applying {
                    "Applying\u{2026}"
                } else {
                    "Apply Changes"
                };
                if primary_button(ui, label).clicked() && !applying {
                    result = YamlChoice::Apply(false);
                }
                ui.add_space(8.0);
                if danger_button(ui, "Force Apply").clicked() && !applying {
                    result = YamlChoice::Apply(true);
                }
                ui.add_space(8.0);
                if secondary_button(ui, '\0', "Cancel").clicked() && !applying {
                    result = YamlChoice::Cancel;
                }
            });
        });
    result
}

/// The port-forward dialog's outcome.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ForwardChoice {
    Start,
    Cancel,
    None,
}

/// Vue's `PortForwardDialog`: what to forward, and which ports. An empty local
/// port means Auto, which is how the Vue dialog treats it.
pub fn forward_dialog(
    ctx: &egui::Context,
    dialog: &mut crate::portforward::Dialog,
) -> ForwardChoice {
    let mut choice = ForwardChoice::None;
    egui::Modal::new(egui::Id::new("forward-dialog"))
        .backdrop_color(Color32::from_black_alpha(140))
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_SECONDARY)
                .stroke(Stroke::new(1.0, theme::EDGE))
                .corner_radius(0.0)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(
                RichText::new("New port forward")
                    .size(15.0)
                    .strong()
                    .color(theme::FG),
            );
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("Resource").size(12.0).color(theme::FG_MUTED));
                for (target, label) in [
                    (crate::portforward::Target::Pod, "Pod"),
                    (crate::portforward::Target::Service, "Service"),
                ] {
                    let selected = dialog.target == target;
                    if ui
                        .selectable_label(selected, RichText::new(label).size(12.0))
                        .clicked()
                    {
                        dialog.target = target;
                    }
                }
            });
            ui.add_space(6.0);
            field(ui, "Name", &mut dialog.name);
            field(ui, "Namespace", &mut dialog.namespace);
            field(ui, "Local port", &mut dialog.local_port);
            ui.label(
                RichText::new("leave the local port empty to pick a free one")
                    .size(10.5)
                    .color(theme::FG_MUTED),
            );
            ui.add_space(4.0);
            field(ui, "Remote port", &mut dialog.remote_port);
            if !dialog.ports.is_empty() {
                ui.label(
                    RichText::new(format!("{} listens on {}", dialog.name, dialog.ports))
                        .size(10.5)
                        .color(theme::FG_MUTED),
                );
            }

            if let Some(error) = &dialog.error {
                ui.add_space(8.0);
                ui.label(RichText::new(error).size(11.5).color(theme::BAD));
            }

            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let start = ui.add(
                    egui::Button::new(
                        RichText::new("Start forward")
                            .size(12.5)
                            .color(theme::SURFACE),
                    )
                    .fill(theme::ACCENT),
                );
                if start.clicked() {
                    choice = ForwardChoice::Start;
                }
                ui.add_space(8.0);
                if ui
                    .add(egui::Button::new(RichText::new("Cancel").size(12.5)))
                    .clicked()
                {
                    choice = ForwardChoice::Cancel;
                }
            });
        });
    choice
}

/// One labelled text field of the forward dialog.
fn field(ui: &mut Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(theme::FG_MUTED));
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(240.0)
                .font(egui::TextStyle::Monospace),
        );
    });
    ui.add_space(4.0);
}

/// Destructive-action confirmation, the equivalent of the Vue `Dialog` +
/// `dialogStore.open({ variant: 'danger' })` flow.
pub fn confirm_dialog(
    ctx: &egui::Context,
    open: bool,
    title: &str,
    message: &str,
    confirm_label: &str,
    danger: bool,
) -> Confirm {
    if !open {
        return Confirm::None;
    }
    let mut result = Confirm::None;
    egui::Modal::new(egui::Id::new("confirm-dialog"))
        .backdrop_color(Color32::from_black_alpha(140))
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_SECONDARY)
                .stroke(Stroke::new(1.0, theme::EDGE))
                .corner_radius(0.0)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(title).size(15.0).strong().color(theme::FG));
            ui.add_space(8.0);
            ui.label(RichText::new(message).size(13.0).color(theme::FG_SECONDARY));
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let fill = if danger { theme::BAD } else { theme::ACCENT };
                let label = RichText::new(confirm_label)
                    .color(theme::SURFACE)
                    .size(13.0);
                let galley = ui.painter().layout_no_wrap(
                    confirm_label.to_string(),
                    FontId::proportional(13.0),
                    theme::SURFACE,
                );
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(galley.size().x + 28.0, 30.0), Sense::click());
                let fill = if resp.hovered() {
                    theme::alpha(fill, 0.85)
                } else {
                    fill
                };
                ui.painter().rect_filled(rect, 0.0, fill);
                ui.painter()
                    .galley(rect.center() - galley.size() / 2.0, galley, theme::SURFACE);
                let _ = label;
                if resp.clicked() {
                    result = Confirm::Accept;
                }
                ui.add_space(8.0);
                if secondary_button(ui, '\0', "Cancel").clicked() {
                    result = Confirm::Cancel;
                }
            });
        });
    result
}

/// Scale dialog body: −/value/+ like `DeploymentsView.vue`, with the
/// "scaling to 0 stops all pods" warning.
pub fn scale_dialog(ctx: &egui::Context, name: &str, current: i64, value: &mut i64) -> Confirm {
    let mut result = Confirm::None;
    egui::Modal::new(egui::Id::new("scale-dialog"))
        .backdrop_color(Color32::from_black_alpha(140))
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_SECONDARY)
                .stroke(Stroke::new(1.0, theme::EDGE))
                .corner_radius(0.0)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(
                RichText::new("Scale Deployment")
                    .size(15.0)
                    .strong()
                    .color(theme::FG),
            );
            ui.add_space(2.0);
            ui.label(RichText::new(name).size(12.0).color(theme::FG_MUTED));
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    // centre the − n + row
                    let total = 40.0 + 8.0 + 80.0 + 8.0 + 40.0;
                    let avail = ui.available_width();
                    ui.add_space(((avail - total) / 2.0).max(0.0));
                    if square_button(ui, crate::icons::MINUS, *value > 0).clicked() {
                        *value = (*value - 1).max(0);
                    }
                    let mut text = value.to_string();
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(80.0, 40.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, theme::SURFACE_TERTIARY);
                    ui.painter().rect_stroke(
                        rect,
                        0.0,
                        Stroke::new(1.0, theme::EDGE),
                        egui::StrokeKind::Inside,
                    );
                    let font = FontId::monospace(18.0);
                    let galley = ui.painter().layout_no_wrap(text.clone(), font, theme::FG);
                    ui.painter()
                        .galley(rect.center() - galley.size() / 2.0, galley, theme::FG);
                    let resp = ui.interact(rect, ui.id().with("scale-input"), Sense::click());
                    if resp.clicked() {
                        // one click = one replica, which is what the Vue number
                        // input does on arrow-up; the − / + buttons are primary
                        *value += 1;
                    }
                    text.clear();
                    if square_button(ui, crate::icons::PLUS, true).clicked() {
                        *value += 1;
                    }
                });
                ui.add_space(12.0);
                let arrow = if *value != current {
                    format!("  \u{2192}  {value} replicas")
                } else {
                    String::new()
                };
                ui.label(
                    RichText::new(format!("Current: {current} replicas{arrow}"))
                        .size(12.0)
                        .color(theme::FG_MUTED),
                );
                if *value == 0 {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "{}  Scaling to 0 will stop all pods",
                            crate::icons::ALERT
                        ))
                        .size(12.0)
                        .color(theme::WARN),
                    );
                }
            });
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if *value == 0 {
                    "Scale to Zero"
                } else {
                    "Scale"
                };
                let fill = if *value == 0 {
                    theme::BAD
                } else {
                    theme::ACCENT
                };
                let enabled = *value != current;
                let galley = ui.painter().layout_no_wrap(
                    label.to_string(),
                    FontId::proportional(13.0),
                    theme::SURFACE,
                );
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(galley.size().x + 28.0, 30.0), Sense::click());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    if enabled {
                        fill
                    } else {
                        theme::SURFACE_TERTIARY
                    },
                );
                ui.painter()
                    .galley(rect.center() - galley.size() / 2.0, galley, theme::SURFACE);
                if resp.clicked() && enabled {
                    result = Confirm::Accept;
                }
                ui.add_space(8.0);
                if secondary_button(ui, '\0', "Cancel").clicked() {
                    result = Confirm::Cancel;
                }
            });
        });
    result
}

fn square_button(ui: &mut Ui, icon: char, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::SURFACE_HOVER);
    }
    ui.painter().rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, theme::EDGE),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(18.0),
        if enabled { theme::FG } else { theme::FG_MUTED },
    );
    resp
}
