//! Right-hand resource drawer: General / Logs / Conditions / YAML.
//!
//! Mirrors `ResourceDrawerGeneric.vue`. The General tab is field-driven
//! (whatever the object's status/spec scalars are) rather than hard-coded per
//! kind, which is what makes it work for CRDs as well as core types.

use eframe::egui::{self, Align2, FontId, RichText, Ui};
use kube::core::DynamicObject;
use serde_json::Value;

use crate::kinds::{Action, Kind, LogsSupport, human_age, parse_secs};
use crate::{App, DrawerTab, cluster, icons, notice, theme, widgets};

/// The drawer's width. It floats over the view, so this is a starting width
/// rather than a column in a split: dragging its left edge changes only how much
/// of the table the panel covers. The table keeps its own widths either way,
/// which is the whole point of floating the drawer instead of reserving a
/// column — so the resize costs no reflow and needs no layout gymnastics.
pub const DEFAULT_W: f32 = 520.0;
/// Narrower than this and the monospace log lines turn to slivers; wider than
/// `MAX_FRACTION` of the view and there is no table left to look at.
pub const MIN_W: f32 = 380.0;
/// A hard ceiling as well as the view-relative one: on a very wide monitor 72%
/// of the screen is not a sensible drawer.
pub const MAX_W: f32 = 900.0;
const MAX_FRACTION: f32 = 0.72;
/// Width of the invisible drag strip on the drawer's left edge.
const GRIP_W: f32 = 6.0;
/// The drawer's own padding: the header's title, the tab strip and the content
/// all start here, as `px-4` and `p-4` do in the Vue.
const DRAWER_PAD: f32 = 16.0;

/// Status keys worth showing first (kubectl-style ordering).
const STATUS_ORDER: &[&str] = &[
    "phase",
    "replicas",
    "readyReplicas",
    "availableReplicas",
    "updatedReplicas",
    "currentReplicas",
    "replicasReady",
    "numberReady",
    "numberAvailable",
    "currentNumberScheduled",
    "desiredNumberScheduled",
    "currentHealthy",
    "desiredHealthy",
    "disruptionsAllowed",
    "succeeded",
    "failed",
    "active",
    "conditions",
];

/// Spec keys worth showing first.
const SPEC_ORDER: &[&str] = &[
    "replicas",
    "serviceName",
    "type",
    "clusterIP",
    "selector",
    "ports",
    "schedule",
    "suspend",
    "completions",
    "parallelism",
    "storageClassName",
    "accessModes",
    "capacity",
    "volumeName",
    "ingressClassName",
    "minAvailable",
    "maxUnavailable",
];

pub fn show(app: &mut App, ui: &mut Ui) {
    let Some((kind, namespace, name, object)) = app.selection.as_ref().map(|s| {
        (
            s.kind,
            s.namespace.clone(),
            s.name.clone(),
            s.object.clone(),
        )
    }) else {
        return;
    };

    let conditions = conditions_of(&object);
    let has_logs = kind.logs != LogsSupport::None;
    // Only the kinds whose payload is key/value pairs get the data tab; a
    // secret's values are base64 on the wire, which is exactly what makes them
    // unreadable without it.
    let has_data = matches!(kind.id, "secrets" | "configmaps");
    let object_events = app.object_events.clone();
    let timeline = app.timeline.clone();

    let (tab_labels, tabs) = tab_strip(has_logs, conditions.len(), has_data);
    let active = tabs.iter().position(|t| *t == app.drawer_tab).unwrap_or(0);
    // The tab actually on screen. The remembered tab may be one this object does
    // not have — Logs on a kind that has none, Conditions on an object without
    // any — and then the first tab is shown. The content follows this rather
    // than the remembered tab, so the highlight and the body can never disagree.
    let shown = tabs.get(active).copied().unwrap_or(DrawerTab::General);

    let mut close = false;
    let mut requested_tab: Option<DrawerTab> = None;
    let mut drawer_action: Option<GeneralAction> = None;
    let mut tab_action: Option<GeneralAction> = None;
    // The "Copied" note fades on its own, so keep painting while it is up.
    let just_copied = app
        .copied_at
        .map(|at| at.elapsed() < std::time::Duration::from_secs(3))
        .unwrap_or(false);
    if just_copied {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }

    // The drawer floats over the view rather than taking a column from it. A
    // panel that reserves space makes the table reflow every time the drawer
    // opens — every column narrows, names truncate, and the widths jump back
    // when it closes. Laid over the top, the table keeps the widths it has and
    // the drawer covers part of it for as long as it is open.
    let view = ui.max_rect();
    let max_w = (view.width() * MAX_FRACTION).clamp(MIN_W, MAX_W);
    let width = app.drawer_width.clamp(MIN_W, max_w);
    let size = egui::vec2(width, view.height());
    let origin = egui::pos2(view.right() - size.x, view.top());
    egui::Area::new(egui::Id::new("resource_drawer"))
        .order(egui::Order::Foreground)
        .fixed_pos(origin)
        .show(ui.ctx(), |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            // Paint the surface across the whole rect the drawer was given.
            // A frame shrinks to its content, so a drawer whose fill came from
            // the frame covered only its header and tab strip — the view showed
            // through underneath, which is what a floating panel must never do.
            let rect = egui::Rect::from_min_size(ui.min_rect().min, size);
            ui.painter().add(
                egui::epaint::Shadow {
                    offset: [-6, 0],
                    blur: 18,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(90),
                }
                .as_shape(rect, 0.0),
            );
            ui.painter()
                .rect_filled(rect, 0.0, theme::SURFACE_SECONDARY);
            ui.painter().rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, theme::EDGE),
                egui::StrokeKind::Inside,
            );

            // The drag handle sits inside the drawer's own layer. A strip to the
            // left of the panel looks the same but loses the hit test: the rows
            // underneath are interactive too, and they are what the pointer
            // would be handed. Content starts at DRAWER_PAD, so a strip this
            // narrow never covers anything.
            let grip = egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.left() + GRIP_W, rect.bottom()),
            );
            let grip_response =
                ui.interact(grip, egui::Id::new("drawer_resize"), egui::Sense::drag());
            if grip_response.hovered() || grip_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                // a hairline on the edge, so the drag target is visible
                ui.painter().line_segment(
                    [grip.center_top(), grip.center_bottom()],
                    egui::Stroke::new(1.0, theme::EDGE_HOVER),
                );
            }
            if grip_response.dragged() {
                // dragging left widens: the new width is the old one minus the
                // delta, clamped to what the view can afford
                app.drawer_width =
                    (app.drawer_width - grip_response.drag_delta().x).clamp(MIN_W, max_w);
            }
            if grip_response.drag_stopped() {
                // one write per drag, not one per frame
                app.save_settings();
            }
            egui::Frame::new()
                .inner_margin(egui::Margin::ZERO)
                .show(ui, |ui| {
                    match drawer_header(ui, kind, &name, just_copied) {
                        HeaderAction::None => {}
                        HeaderAction::Close => close = true,
                        HeaderAction::Delete => drawer_action = Some(GeneralAction::Delete),
                    }
                    if let Some(idx) = widgets::tabs(ui, &tab_labels, active) {
                        requested_tab = tabs.get(idx).copied();
                    }

                    // the content is pinned to the drawer's own rect and inset by
                    // the drawer's padding, so every tab lines up with the header
                    let content_rect = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + DRAWER_PAD, ui.cursor().top().max(rect.top())),
                        egui::pos2(rect.right() - DRAWER_PAD, rect.max.y),
                    );
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
                    child.set_clip_rect(content_rect);
                    match shown {
                        DrawerTab::General => {
                            tab_action = general_tab(
                                &mut child,
                                kind,
                                &object,
                                &namespace,
                                &object_events,
                                timeline.as_ref(),
                            );
                        }
                        DrawerTab::Logs => logs_tab(&mut child, app),
                        DrawerTab::Conditions => conditions_tab(&mut child, &conditions),
                        DrawerTab::Data => {
                            tab_action =
                                data_tab(&mut child, app, kind, &namespace, &name, &object);
                        }
                        DrawerTab::Yaml => yaml_tab(&mut child, app),
                    }
                });
        });

    if close {
        app.close_drawer();
        return;
    }
    if let Some(tab) = requested_tab {
        app.set_drawer_tab(tab);
    }
    if let Some(action) = drawer_action.or(tab_action) {
        match action {
            GeneralAction::Logs(container) => {
                if let Some(container) = container {
                    app.select_log_container(container);
                }
                app.set_drawer_tab(DrawerTab::Logs);
            }
            GeneralAction::Tab(tab) => app.set_drawer_tab(tab),
            GeneralAction::Copy(text) => {
                ui.ctx().copy_text(text);
                app.copied_at = Some(std::time::Instant::now());
            }
            GeneralAction::EditData => {
                app.begin_data_edit(kind, &namespace, &name, &object);
            }
            GeneralAction::SaveData => app.save_data_edit(),
            GeneralAction::CancelData => app.cancel_data_edit(),
            GeneralAction::Delete => {
                app.begin_action(kind, &format!("{namespace}/{name}"), Action::Delete);
            }
        }
    }
}

/// A secret's or configmap's key/value pairs, decoded. Reading them is the point
/// of the tab; editing them is a step the user takes deliberately, because
/// writing a secret back is a real change to a live object.
fn data_tab(
    ui: &mut Ui,
    app: &mut App,
    kind: &'static Kind,
    namespace: &str,
    name: &str,
    object: &DynamicObject,
) -> Option<GeneralAction> {
    const ROW_H: f32 = 22.0;

    let mut action = None;
    let is_secret = kind.id == "secrets";
    let entries = cluster::data_entries(object, is_secret);
    let editing = app
        .data_edit
        .as_ref()
        .is_some_and(|edit| edit.name == name && edit.namespace == namespace);

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "{} {}",
                entries.len(),
                if entries.len() == 1 { "key" } else { "keys" }
            ))
            .size(11.0)
            .color(theme::FG_MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if editing {
                if widgets::secondary_button(ui, icons::CLOSE, "Cancel").clicked() {
                    action = Some(GeneralAction::CancelData);
                }
                if widgets::secondary_button(ui, icons::CONTENT_SAVE, "Save").clicked() {
                    action = Some(GeneralAction::SaveData);
                }
            } else if widgets::secondary_button(ui, icons::PENCIL_OUTLINE, "Edit").clicked() {
                action = Some(GeneralAction::EditData);
            }
        });
    });
    ui.add_space(8.0);

    if entries.is_empty() {
        widgets::empty_state(ui, icons::KEY_OUTLINE, "No data");
        return action;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, (key, value)) in entries.iter().enumerate() {
                let key = key.clone();
                let value = value.clone();

                // The key is a label above its value, not a column beside it: the
                // value is the thing being read, and a second column in a 520px
                // drawer leaves it about a third of the width.
                ui.label(
                    RichText::new(&key)
                        .monospace()
                        .size(11.5)
                        .color(theme::FG_MUTED),
                );
                ui.add_space(4.0);

                if editing {
                    let mut text = app
                        .data_edit
                        .as_ref()
                        .and_then(|edit| edit.values.get(index))
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default();
                    let response = ui.add_sized(
                        egui::vec2(ui.available_width(), ROW_H),
                        egui::TextEdit::singleline(&mut text).font(egui::TextStyle::Monospace),
                    );
                    // an editable value needs an edge to read as one; the
                    // focused field takes the accent, as the app's inputs do
                    ui.painter().rect_stroke(
                        response.rect,
                        0.0,
                        egui::Stroke::new(
                            1.0,
                            if response.has_focus() {
                                theme::ACCENT
                            } else {
                                theme::EDGE
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    if response.changed()
                        && let Some(edit) = app.data_edit.as_mut()
                        && let Some(slot) = edit.values.get_mut(index)
                    {
                        slot.1 = text;
                    }
                } else {
                    // The shape of an input, so the value reads as one — lit
                    // rather than dimmed, because a secret's data is there to be
                    // read. Clicking copies it, as it always did.
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), ROW_H),
                        egui::Sense::click(),
                    );
                    ui.painter().rect_filled(rect, 0.0, theme::SURFACE);
                    ui.painter().rect_stroke(
                        rect,
                        0.0,
                        egui::Stroke::new(1.0, theme::EDGE),
                        egui::StrokeKind::Inside,
                    );
                    let mut inner = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect.shrink2(egui::vec2(8.0, 0.0)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    inner.add(
                        egui::Label::new(
                            RichText::new(&value)
                                .monospace()
                                .size(11.5)
                                .color(theme::FG),
                        )
                        .truncate(),
                    );
                    if response.clicked() {
                        action = Some(GeneralAction::Copy(value.clone()));
                    }
                    response.on_hover_text(&value);
                }
                ui.add_space(10.0);
            }
        });
    action
}

/// The tab strip: the labels `widgets::tabs` draws, and the tab each index
/// stands for. They are built together so they cannot drift apart.
fn tab_strip(
    has_logs: bool,
    condition_count: usize,
    has_data: bool,
) -> (Vec<(String, Option<String>)>, Vec<DrawerTab>) {
    let mut labels = vec![("General".to_string(), None)];
    let mut tabs = vec![DrawerTab::General];
    if has_logs {
        labels.push(("Logs".to_string(), None));
        tabs.push(DrawerTab::Logs);
    }
    if condition_count > 0 {
        labels.push(("Conditions".to_string(), Some(condition_count.to_string())));
        tabs.push(DrawerTab::Conditions);
    }
    if has_data {
        labels.push(("Data".to_string(), None));
        tabs.push(DrawerTab::Data);
    }
    labels.push(("YAML".to_string(), None));
    tabs.push(DrawerTab::Yaml);
    (labels, tabs)
}

/// The drawer's title bar: kind glyph, monospace name, then the actions and the
/// close button as small icons on the right.
enum HeaderAction {
    None,
    Close,
    Delete,
}

fn drawer_header(ui: &mut Ui, kind: &Kind, name: &str, just_copied: bool) -> HeaderAction {
    let mut action = HeaderAction::None;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 40.0), egui::Sense::hover());
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, theme::EDGE),
    );
    ui.painter().text(
        egui::pos2(rect.left() + DRAWER_PAD, rect.center().y),
        Align2::LEFT_CENTER,
        kind.icon,
        FontId::proportional(16.0),
        theme::ACCENT,
    );

    // the icons own the right end of the header, so the title is clipped short
    // of them instead of running underneath
    let icons_w = if just_copied { 210.0 } else { 150.0 };
    ui.painter()
        .with_clip_rect(egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.right() - icons_w, rect.bottom()),
        ))
        .text(
            egui::pos2(rect.left() + DRAWER_PAD + 24.0, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            FontId::monospace(13.0),
            theme::FG,
        );

    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
        egui::pos2(rect.right() - icons_w, rect.top()),
        rect.max,
    )));
    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::icon_button(ui, crate::icons::CLOSE, theme::FG_SECONDARY, theme::FG).clicked() {
            action = HeaderAction::Close;
        }
        if kind.actions.contains(&Action::Delete)
            && widgets::icon_button(
                ui,
                crate::icons::DELETE_OUTLINE,
                theme::FG_SECONDARY,
                theme::BAD,
            )
            .on_hover_text(format!("Delete {}", kind.kind))
            .clicked()
        {
            action = HeaderAction::Delete;
        }

        if just_copied {
            ui.label(RichText::new("Copied").size(11.0).color(theme::OK));
        }
    });
    action
}

// ---------------------------------------------------------------------------
// General
// ---------------------------------------------------------------------------

/// What the drawer asks the app to do. The drawer draws and the app acts, so a
/// click here reaches the same path a table's row action does.
pub enum GeneralAction {
    /// Open the Logs tab, on a specific container when one was clicked.
    Logs(Option<String>),
    /// Switch to a tab.
    Tab(DrawerTab),
    /// Put text on the clipboard.
    Copy(String),
    /// Open the data tab's editor on this secret or configmap.
    EditData,
    /// Write the edited values back to the cluster.
    SaveData,
    /// Leave the editor, discarding the typed values.
    CancelData,
    /// Delete this object, through the usual confirmation.
    Delete,
}

/// How loudly a value should speak.
#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Quiet,
    Ok,
    Warn,
    Bad,
}

impl Tone {
    fn color(self) -> Option<egui::Color32> {
        match self {
            Tone::Quiet => None,
            Tone::Ok => Some(theme::OK),
            Tone::Warn => Some(theme::WARN),
            Tone::Bad => Some(theme::BAD),
        }
    }
}

/// The General tab, in the order a person wants it: what state it is in, what is
/// running, what it costs, why it might not be ready, then the raw material.
/// Labels and annotations are collapsed — they are most of the text and least of
/// the answer, and annotations are where container-runtime noise piles up.
fn general_tab(
    ui: &mut Ui,
    kind: &Kind,
    object: &kube::core::DynamicObject,
    namespace: &str,
    events: &[crate::cluster::EventRow],
    timeline: Option<&Result<crate::prometheus::Timeline, String>>,
) -> Option<GeneralAction> {
    let mut action = None;
    let conditions = conditions_of(object);
    egui::ScrollArea::vertical()
        .id_salt("drawer_general")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(12.0);
            block_title(ui, "At a glance");
            glance_grid(ui, &glance(kind, object, namespace));

            if kind.id == "pods" {
                containers_block(ui, object, &mut action);
            }
            if let Some(timeline) = timeline {
                usage_block(ui, timeline);
            }
            conditions_block(ui, &conditions, &mut action);

            let labels = object.metadata.labels.clone().unwrap_or_default();
            let annotations = object.metadata.annotations.clone().unwrap_or_default();
            metadata_block(ui, "Labels", &labels, &mut action);
            metadata_block(ui, "Annotations", &annotations, &mut action);
            raw_block(ui, object);
            events_block(ui, events);
            ui.add_space(16.0);
        });
    action
}

/// A section title with a hairline above it, so the blocks read as blocks.
fn block_title(ui: &mut Ui, title: &str) {
    ui.add_space(12.0);
    let width = (ui.available_width() - 20.0).max(0.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        egui::Stroke::new(1.0, theme::EDGE),
    );
    ui.add_space(6.0);
    section(ui, title);
}

fn dot(ui: &mut Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 14.0), egui::Sense::hover());
    ui.painter()
        .circle_filled(egui::pos2(rect.left() + 4.0, rect.center().y), 3.5, color);
}

/// The facts worth knowing before anything else, per kind.
fn glance(
    kind: &Kind,
    object: &kube::core::DynamicObject,
    namespace: &str,
) -> Vec<(&'static str, String, Tone)> {
    let mut out: Vec<(&'static str, String, Tone)> = Vec::new();
    let mut push = |label: &'static str, value: Option<String>, tone: Tone| {
        if let Some(value) = value {
            out.push((label, value, tone));
        }
    };

    let statuses = object
        .data
        .pointer("/status/containerStatuses")
        .and_then(Value::as_array);
    let total = object
        .data
        .pointer("/spec/containers")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let ready = statuses
        .map(|list| {
            list.iter()
                .filter(|c| {
                    c.pointer("/ready")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    let restarts: i64 = statuses
        .map(|list| {
            list.iter()
                .filter_map(|c| c.pointer("/restartCount").and_then(Value::as_i64))
                .sum()
        })
        .unwrap_or(0);

    match kind.id {
        "pods" => {
            push(
                "Ready",
                Some(format!("{ready}/{total}")),
                if total > 0 && ready == total {
                    Tone::Ok
                } else {
                    Tone::Warn
                },
            );
            push(
                "Restarts",
                Some(restarts.to_string()),
                match restarts {
                    0 => Tone::Quiet,
                    1..=9 => Tone::Warn,
                    _ => Tone::Bad,
                },
            );
            let phase = object
                .data
                .pointer("/status/phase")
                .and_then(Value::as_str)
                .map(str::to_string);
            let tone = match phase.as_deref() {
                Some("Running") => Tone::Ok,
                Some("Succeeded") => Tone::Quiet,
                Some("Failed") => Tone::Bad,
                _ => Tone::Warn,
            };
            push("Phase", phase, tone);
            push("Node", str_at(object, "/spec/nodeName"), Tone::Quiet);
            push("Pod IP", str_at(object, "/status/podIP"), Tone::Quiet);
            push("QoS", str_at(object, "/status/qosClass"), Tone::Quiet);
        }
        "deployments" | "statefulsets" | "daemonsets" | "replicasets" => {
            let desired = object
                .data
                .pointer("/spec/replicas")
                .and_then(Value::as_i64)
                .or_else(|| {
                    object
                        .data
                        .pointer("/status/desiredNumberScheduled")
                        .and_then(Value::as_i64)
                })
                .unwrap_or(0);
            let ready = object
                .data
                .pointer("/status/readyReplicas")
                .and_then(Value::as_i64)
                .or_else(|| {
                    object
                        .data
                        .pointer("/status/numberReady")
                        .and_then(Value::as_i64)
                })
                .unwrap_or(0);
            push(
                "Ready",
                Some(format!("{ready}/{desired}")),
                if desired > 0 && ready == desired {
                    Tone::Ok
                } else {
                    Tone::Warn
                },
            );
            push(
                "Updated",
                object
                    .data
                    .pointer("/status/updatedReplicas")
                    .and_then(Value::as_i64)
                    .or_else(|| {
                        object
                            .data
                            .pointer("/status/updatedNumberScheduled")
                            .and_then(Value::as_i64)
                    })
                    .map(|n| n.to_string()),
                Tone::Quiet,
            );
            push(
                "Available",
                object
                    .data
                    .pointer("/status/availableReplicas")
                    .and_then(Value::as_i64)
                    .or_else(|| {
                        object
                            .data
                            .pointer("/status/numberAvailable")
                            .and_then(Value::as_i64)
                    })
                    .map(|n| n.to_string()),
                Tone::Quiet,
            );
            push(
                "Strategy",
                str_at(object, "/spec/strategy/type")
                    .or_else(|| str_at(object, "/spec/updateStrategy/type")),
                Tone::Quiet,
            );
            push("Images", images_of(object), Tone::Quiet);
        }
        "services" => {
            push("Type", str_at(object, "/spec/type"), Tone::Quiet);
            push("Cluster IP", str_at(object, "/spec/clusterIP"), Tone::Quiet);
            push("Ports", service_ports(object), Tone::Quiet);
            push("Selector", selector_of(object), Tone::Quiet);
            push(
                "External IP",
                object
                    .data
                    .pointer("/status/loadBalancer/ingress/0/ip")
                    .or_else(|| {
                        object
                            .data
                            .pointer("/status/loadBalancer/ingress/0/hostname")
                    })
                    .and_then(Value::as_str)
                    .map(str::to_string),
                Tone::Quiet,
            );
        }
        "nodes" => {
            let ready = object
                .data
                .pointer("/status/conditions")
                .and_then(Value::as_array)
                .and_then(|list| {
                    list.iter()
                        .find(|c| c.get("type").and_then(Value::as_str) == Some("Ready"))
                })
                .and_then(|c| c.get("status").and_then(Value::as_str))
                .map(str::to_string);
            push(
                "Status",
                ready.clone(),
                if ready.as_deref() == Some("True") {
                    Tone::Ok
                } else {
                    Tone::Bad
                },
            );
            let roles: Vec<String> = object
                .metadata
                .labels
                .clone()
                .unwrap_or_default()
                .keys()
                .filter_map(|k| {
                    k.strip_prefix("node-role.kubernetes.io/")
                        .map(str::to_string)
                })
                .filter(|r| !r.is_empty())
                .collect();
            push(
                "Roles",
                (!roles.is_empty()).then(|| roles.join(", ")),
                Tone::Quiet,
            );
            push(
                "Version",
                str_at(object, "/status/nodeInfo/kubeletVersion"),
                Tone::Quiet,
            );
            push("CPU", str_at(object, "/status/capacity/cpu"), Tone::Quiet);
            push(
                "Memory",
                str_at(object, "/status/capacity/memory"),
                Tone::Quiet,
            );
        }
        "cronjobs" => {
            push("Schedule", str_at(object, "/spec/schedule"), Tone::Quiet);
            push(
                "Suspended",
                object
                    .data
                    .pointer("/spec/suspend")
                    .and_then(Value::as_bool)
                    .filter(|s| *s)
                    .map(|_| "yes".to_string()),
                Tone::Warn,
            );
            push(
                "Last schedule",
                object
                    .data
                    .pointer("/status/lastScheduleTime")
                    .and_then(Value::as_str)
                    .map(|t| t.to_string()),
                Tone::Quiet,
            );
        }
        "jobs" => {
            push(
                "Completions",
                object
                    .data
                    .pointer("/status/succeeded")
                    .and_then(Value::as_i64)
                    .map(|s| {
                        let want = object
                            .data
                            .pointer("/spec/completions")
                            .and_then(Value::as_i64)
                            .unwrap_or(1);
                        format!("{s}/{want}")
                    }),
                Tone::Quiet,
            );
        }
        _ => {}
    }

    if !namespace.is_empty() {
        push("Namespace", Some(namespace.to_string()), Tone::Quiet);
    }
    if let Some(created) = object.metadata.creation_timestamp.as_ref() {
        push(
            "Age",
            Some(format!("{} ago", human_age(created.0.as_second()))),
            Tone::Quiet,
        );
    }
    if let Some(owners) = object.metadata.owner_references.as_ref()
        && !owners.is_empty()
    {
        push(
            "Owner",
            Some(
                owners
                    .iter()
                    .map(|o| format!("{}/{}", o.kind, o.name))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Tone::Quiet,
        );
    }
    out.truncate(10);
    out
}

fn str_at(object: &kube::core::DynamicObject, pointer: &str) -> Option<String> {
    object
        .data
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn images_of(object: &kube::core::DynamicObject) -> Option<String> {
    let list = object
        .data
        .pointer("/spec/template/spec/containers")
        .and_then(Value::as_array)?;
    let images: Vec<String> = list
        .iter()
        .filter_map(|c| c.get("image").and_then(Value::as_str))
        .map(short_image)
        .collect();
    (!images.is_empty()).then(|| images.join(", "))
}

fn selector_of(object: &kube::core::DynamicObject) -> Option<String> {
    let map = object.data.pointer("/spec/selector")?.as_object()?;
    let text = map
        .iter()
        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(", ");
    (!text.is_empty()).then_some(text)
}

fn service_ports(object: &kube::core::DynamicObject) -> Option<String> {
    let list = object.data.pointer("/spec/ports")?.as_array()?;
    let text = list
        .iter()
        .map(|p| {
            let port = p.get("port").and_then(Value::as_i64).unwrap_or(0);
            let target = match p.get("targetPort") {
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::String(s)) => s.clone(),
                _ => port.to_string(),
            };
            let protocol = p.get("protocol").and_then(Value::as_str).unwrap_or("TCP");
            format!("{port}→{target}/{protocol}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    (!text.is_empty()).then_some(text)
}

/// Image without the registry host, which is noise in a one-line row.
fn short_image(image: &str) -> String {
    let short = image
        .strip_prefix("docker.io/")
        .or_else(|| image.strip_prefix("index.docker.io/"))
        .unwrap_or(image);
    ellipsize(short, 34)
}

/// Cut text to `cap` characters, marking the cut.
fn ellipsize(text: &str, cap: usize) -> String {
    if text.chars().count() > cap {
        let cut: String = text.chars().take(cap.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    }
}

/// Two columns of label/value pairs; a toned value leads with a dot. Values are
/// capped so a long owner or selector cannot widen the grid past the drawer and
/// push later rows off the edge.
fn glance_grid(ui: &mut Ui, pairs: &[(&'static str, String, Tone)]) {
    egui::Grid::new("drawer_glance")
        .num_columns(4)
        .spacing([16.0, 7.0])
        .show(ui, |ui| {
            for (index, (label, value, tone)) in pairs.iter().enumerate() {
                ui.label(RichText::new(*label).size(11.0).color(theme::FG_MUTED));
                let shown = if value.chars().count() > 28 {
                    let cut: String = value.chars().take(27).collect();
                    format!("{cut}…")
                } else {
                    value.clone()
                };
                let text = RichText::new(shown).size(11.5).monospace().color(theme::FG);
                match tone.color() {
                    Some(color) => {
                        ui.horizontal(|ui| {
                            dot(ui, color);
                            ui.label(text).on_hover_text(value);
                        });
                    }
                    None => {
                        ui.label(text).on_hover_text(value);
                    }
                }
                if index % 2 == 1 {
                    ui.end_row();
                }
            }
            if pairs.len() % 2 == 1 {
                ui.end_row();
            }
        });
}

/// One row per container: state, image, restarts, and the way to its logs.
///
/// Init containers get their own heading above the app containers: they run
/// before them, and their state means the opposite thing — "Completed" is the
/// good outcome, which is why `container_state` tones it quiet rather than red.
/// Ephemeral (`kubectl debug`) containers get a heading below. Both are missing
/// from `kubectl get pods`' READY column, but not from here.
fn containers_block(
    ui: &mut Ui,
    object: &kube::core::DynamicObject,
    action: &mut Option<GeneralAction>,
) {
    let list = |path: &str| object.data.pointer(path).and_then(Value::as_array);
    let init = list("/spec/initContainers");
    let main = list("/spec/containers");
    let ephemeral = list("/spec/ephemeralContainers");

    let non_empty = |list: Option<&Vec<Value>>| list.map(|l| !l.is_empty()).unwrap_or(false);
    if !non_empty(init) && !non_empty(main) && !non_empty(ephemeral) {
        return;
    }

    for (title, spec, statuses) in [
        (
            "Init containers",
            init,
            list("/status/initContainerStatuses"),
        ),
        ("Containers", main, list("/status/containerStatuses")),
        (
            "Ephemeral containers",
            ephemeral,
            list("/status/ephemeralContainerStatuses"),
        ),
    ] {
        let Some(spec) = spec.filter(|s| !s.is_empty()) else {
            continue;
        };
        block_title(ui, title);
        for container in spec {
            container_row(ui, container, statuses, action);
        }
    }
}

/// One container's line: status dot, name, state, image, restarts, logs link.
fn container_row(
    ui: &mut Ui,
    container: &Value,
    statuses: Option<&Vec<Value>>,
    action: &mut Option<GeneralAction>,
) {
    let name = container
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("container");
    let status = statuses.and_then(|list| {
        list.iter()
            .find(|c| c.get("name").and_then(Value::as_str) == Some(name))
    });
    let ready = status
        .and_then(|s| s.pointer("/ready"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let restarts = status
        .and_then(|s| s.pointer("/restartCount"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let (state, tone) = container_state(status, ready);
    let image = container
        .get("image")
        .and_then(Value::as_str)
        .map(short_image)
        .unwrap_or_default();

    ui.horizontal(|ui| {
        ui.add_space(4.0);
        dot(ui, tone.color().unwrap_or(theme::FG_MUTED));
        ui.label(
            RichText::new(ellipsize(name, 24))
                .size(12.0)
                .monospace()
                .color(theme::FG),
        )
        .on_hover_text(name);
        ui.label(RichText::new(state).size(11.0).color(theme::FG_SECONDARY));
        ui.label(
            RichText::new(ellipsize(&image, 22))
                .size(11.0)
                .monospace()
                .color(theme::FG_MUTED),
        )
        .on_hover_text(&image);
        if restarts > 0 {
            let color = if restarts < 10 {
                theme::WARN
            } else {
                theme::BAD
            };
            ui.label(
                RichText::new(format!("{restarts} rests"))
                    .size(11.0)
                    .color(color),
            )
            .on_hover_text(format!("{restarts} restarts"));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            let link = ui.add(
                egui::Button::new(RichText::new("Logs").size(11.0).color(theme::ACCENT))
                    .frame(false),
            );
            if link.clicked() {
                *action = Some(GeneralAction::Logs(Some(name.to_string())));
            }
        });
    });
}

/// The container's state, in the words `kubectl get pods` uses.
fn container_state(status: Option<&Value>, ready: bool) -> (String, Tone) {
    let Some(state) = status.and_then(|s| s.pointer("/state")) else {
        return ("Pending".to_string(), Tone::Warn);
    };
    if state.get("running").is_some() {
        let tone = if ready { Tone::Ok } else { Tone::Warn };
        return ("Running".to_string(), tone);
    }
    for (key, label) in [("waiting", "Waiting"), ("terminated", "Terminated")] {
        if let Some(inner) = state.get(key) {
            let reason = inner.get("reason").and_then(Value::as_str).unwrap_or(label);
            let tone = if reason == "Completed" {
                Tone::Quiet
            } else {
                Tone::Bad
            };
            return (format!("{label} · {reason}"), tone);
        }
    }
    ("Unknown".to_string(), Tone::Warn)
}

/// Compact usage: a sparkline and the current value per series.
fn usage_block(ui: &mut Ui, timeline: &Result<crate::prometheus::Timeline, String>) {
    block_title(ui, "Usage · last 60m");
    let timeline = match timeline {
        Ok(timeline) => timeline,
        Err(error) => {
            ui.label(RichText::new(error).size(11.0).color(theme::FG_MUTED));
            return;
        }
    };
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        spark(ui, "CPU", &timeline.cpu_cores.points, widgets::fmt_cores);
        ui.add_space(18.0);
        spark(
            ui,
            "Memory",
            &timeline.memory_bytes.points,
            widgets::fmt_bytes,
        );
        ui.add_space(18.0);
        spark(ui, "Disk", &timeline.disk_bytes.points, widgets::fmt_bytes);
    });
}

/// A labelled sparkline with the current value under it.
fn spark(ui: &mut Ui, label: &str, points: &[(i64, f64)], format: fn(f64) -> String) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).size(10.5).color(theme::FG_MUTED));
        if points.len() < 2 {
            ui.label(
                RichText::new("no samples")
                    .size(11.0)
                    .color(theme::FG_MUTED),
            );
            return;
        }
        widgets::sparkline(ui, points, egui::vec2(88.0, 30.0));
        let current = points
            .last()
            .map(|(_, value)| format(*value))
            .unwrap_or_else(|| "\u{2014}".to_string());
        ui.label(
            RichText::new(current)
                .size(11.5)
                .monospace()
                .color(theme::FG),
        );
    });
}

/// Conditions inline: coloured, with how long they have been that way. The
/// Conditions tab keeps the full list with reasons and messages.
fn conditions_block(ui: &mut Ui, conditions: &[Condition], action: &mut Option<GeneralAction>) {
    if conditions.is_empty() {
        return;
    }
    block_title(ui, "Conditions");
    for condition in conditions.iter().take(4) {
        let color = match condition.status.as_str() {
            "True" => theme::OK,
            "False" => theme::WARN,
            _ => theme::WARN,
        };
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            dot(ui, color);
            ui.label(
                RichText::new(&condition.type_)
                    .size(12.0)
                    .monospace()
                    .color(theme::FG),
            );
            ui.label(RichText::new(&condition.status).size(11.0).color(color));
            if let Some(sec) = condition.last_transition {
                ui.label(
                    RichText::new(format!("· {} ago", human_age(sec)))
                        .size(10.5)
                        .color(theme::FG_MUTED),
                );
            }
        });
    }
    if conditions.len() > 4 {
        let more = conditions.len() - 4;
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(format!("{more} more in the Conditions tab"))
                            .size(11.0)
                            .color(theme::FG_MUTED),
                    )
                    .frame(false),
                )
                .clicked()
            {
                *action = Some(GeneralAction::Tab(DrawerTab::Conditions));
            }
        });
    }
}

/// Labels and annotations as a plain key/value list: pills wrap badly in a
/// narrow panel, and a collapsed section hides exactly what the drawer was
/// opened to read. The key keeps a column so the values line up under each
/// other, and a row copies its `key=value` when clicked.
fn metadata_block(
    ui: &mut Ui,
    title: &str,
    map: &std::collections::BTreeMap<String, String>,
    action: &mut Option<GeneralAction>,
) {
    if map.is_empty() {
        return;
    }
    block_title(ui, &format!("{title} ({})", map.len()));
    for (key, value) in map {
        let full = format!("{key}={value}");
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, theme::SURFACE_HOVER);
        }
        let key_w = rect.width() * 0.45;
        let key_rect = egui::Rect::from_min_size(rect.min, egui::vec2(key_w, rect.height()));
        let value_rect =
            egui::Rect::from_min_max(egui::pos2(rect.left() + key_w, rect.top()), rect.max);
        // clipped to the width each is given: a long value is cut, not wrapped
        // into a column one character wide
        ui.painter().with_clip_rect(key_rect).text(
            egui::pos2(key_rect.left(), key_rect.center().y),
            Align2::LEFT_CENTER,
            key,
            FontId::monospace(11.0),
            theme::FG_MUTED,
        );
        ui.painter().with_clip_rect(value_rect).text(
            egui::pos2(value_rect.left() + 10.0, value_rect.center().y),
            Align2::LEFT_CENTER,
            value,
            FontId::monospace(11.0),
            theme::FG,
        );
        if response.clicked() {
            *action = Some(GeneralAction::Copy(full.clone()));
        }
        response.on_hover_text(full);
    }
}

/// Status and spec, collapsed: the raw material, kept but out of the way.
fn raw_block(ui: &mut Ui, object: &kube::core::DynamicObject) {
    let status = scalar_fields(object.data.get("status"), STATUS_ORDER, 12);
    let spec = spec_fields_of(object.data.get("spec"), 10);
    for (title, fields) in [("Status", status), ("Spec", spec)] {
        if fields.is_empty() {
            continue;
        }
        ui.add_space(10.0);
        egui::CollapsingHeader::new(RichText::new(title).size(11.0).color(theme::FG_MUTED))
            .id_salt(("drawer_raw", title))
            .default_open(false)
            .show(ui, |ui| {
                grid(ui, &fields);
            });
    }
}

/// Events about this object, warnings first, with the rest behind a fold.
fn events_block(ui: &mut Ui, events: &[crate::cluster::EventRow]) {
    if events.is_empty() {
        return;
    }
    let mut sorted: Vec<crate::cluster::EventRow> = events.to_vec();
    sorted.sort_by_key(|e| (!e.warning, std::cmp::Reverse(e.at)));
    let shown = sorted.len().min(3);
    ui.add_space(10.0);
    crate::views::events_section(ui, "Events", &sorted[..shown], false, true);
    if sorted.len() > shown {
        egui::CollapsingHeader::new(
            RichText::new(format!("Show all {} events", sorted.len()))
                .size(11.0)
                .color(theme::FG_MUTED),
        )
        .id_salt("drawer_events_all")
        .default_open(false)
        .show(ui, |ui| {
            crate::views::events_section(ui, "", &sorted[shown..], false, true);
        });
    }
}

/// A small uppercase section label, as the Vue's `text-xs uppercase` headers are.
fn section(ui: &mut Ui, title: &str) {
    ui.label(
        RichText::new(title.to_uppercase())
            .size(10.5)
            .color(theme::FG_MUTED),
    );
    ui.add_space(4.0);
}

/// Label/value grid; multi-line values (labels, annotations) get their own row
/// so long maps stay readable instead of being truncated.
fn grid(ui: &mut Ui, fields: &[(String, String)]) {
    for (label, value) in fields {
        let multiline = value.contains('\n') || value.len() > 60;
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("{label}:"))
                    .size(12.0)
                    .color(theme::FG_MUTED),
            );
            if multiline {
                ui.add_space(4.0);
            }
        });
        if multiline {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(
                    RichText::new(value)
                        .size(11.5)
                        .monospace()
                        .color(theme::FG_SECONDARY),
                );
            });
        } else {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new(value).size(12.5).monospace().color(theme::FG));
            });
        }
        ui.add_space(2.0);
    }
}

/// Scalar fields of `status` in display order, with conditions excluded
/// (they have their own tab).
fn scalar_fields(status: Option<&Value>, order: &[&str], cap: usize) -> Vec<(String, String)> {
    let Some(Value::Object(map)) = status else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for key in order {
        if *key == "conditions" {
            continue;
        }
        if let Some(v) = map.get(*key)
            && let Some(text) = scalar_to_text(v)
        {
            out.push((title_case(key), text));
            seen.insert(key.to_string());
        }
    }
    for (key, v) in map {
        if seen.contains(key) || key == "conditions" {
            continue;
        }
        if let Some(text) = scalar_to_text(v) {
            out.push((title_case(key), text));
        }
    }
    out.truncate(cap);
    out
}

/// Spec fields: scalars plus the two compound shapes the Vue drawer renders
/// specially — `selector` (matchLabels) and `ports`.
fn spec_fields_of(spec: Option<&Value>, cap: usize) -> Vec<(String, String)> {
    let Some(Value::Object(map)) = spec else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for key in SPEC_ORDER {
        let Some(v) = map.get(*key) else { continue };
        match *key {
            "selector" => {
                if let Some(labels) = v.get("matchLabels").and_then(Value::as_object) {
                    let text = labels
                        .iter()
                        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push((
                        "Selector".into(),
                        if text.is_empty() {
                            "<empty>".into()
                        } else {
                            text
                        },
                    ));
                    seen.insert((*key).to_string());
                }
            }
            "ports" => {
                if let Some(list) = v.as_array() {
                    let text = list
                        .iter()
                        .map(|p| {
                            let port = p
                                .get("port")
                                .or_else(|| p.get("containerPort"))
                                .and_then(Value::as_i64);
                            let proto = p.get("protocol").and_then(Value::as_str).unwrap_or("TCP");
                            let name = p.get("name").and_then(Value::as_str).unwrap_or("");
                            match (name.is_empty(), port) {
                                (true, Some(p)) => format!("{p}/{proto}"),
                                (false, Some(p)) => format!("{name}: {p}/{proto}"),
                                _ => String::new(),
                            }
                        })
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(", ");
                    if !text.is_empty() {
                        out.push(("Ports".into(), text));
                    }
                    seen.insert((*key).to_string());
                }
            }
            _ => {
                if let Some(text) = scalar_to_text(v) {
                    out.push((title_case(key), text));
                    seen.insert((*key).to_string());
                }
            }
        }
    }

    for (key, v) in map {
        if seen.contains(key) {
            continue;
        }
        if let Some(text) = scalar_to_text(v) {
            out.push((title_case(key), text));
        }
    }
    out.truncate(cap);
    out
}

fn scalar_to_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `readyReplicas` -> `Ready Replicas`
fn title_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for (i, ch) in key.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push(' ');
        }
        if i == 0 {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

fn logs_tab(ui: &mut Ui, app: &mut App) {
    let containers = app.selected_containers();
    let mut switch_to: Option<String> = None;
    let mut restamped: Option<bool> = None;
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        // multi-container pods pick the streamed container here, like Vue's
        // ContainerSelector in the logs toolbar
        if containers.len() > 1 {
            let current = app.log_container.clone().unwrap_or_default();
            if let Some(name) = widgets::select(ui, &current, &containers) {
                switch_to = Some(name);
            }
            ui.add_space(10.0);
        }
        // the toolbar's controls, as the Vue's LogsTab has them
        let mut follow = app.log_follow;
        widgets::checkbox(ui, &mut follow, "Follow");
        app.log_follow = follow;
        ui.add_space(8.0);
        let mut timestamps = app.log_timestamps;
        if widgets::checkbox(ui, &mut timestamps, "Timestamps").clicked() {
            restamped = Some(timestamps);
        }
        ui.add_space(8.0);
        let mut wrap = app.log_wrap;
        widgets::checkbox(ui, &mut wrap, "Wrap");
        app.log_wrap = wrap;
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("{} lines", app.logs.len()))
                .size(11.0)
                .color(theme::FG_MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            if ui
                .button(RichText::new("Copy").size(11.0).color(theme::FG_SECONDARY))
                .clicked()
            {
                ui.ctx().copy_text(app.logs.join("\n"));
            }
        });
    });
    ui.add_space(4.0);

    // Wrapping is done here, not by egui: a wrapped line becomes several uniform
    // rows, so the pane keeps painting only what is on screen. Handing the text
    // to egui to wrap would make rows different heights, which is exactly what
    // the virtualization needs them not to be. With wrapping off, one line is
    // one row and the pane scrolls sideways, as the Vue's `whitespace-pre` does.
    let columns = if app.log_wrap { log_columns(ui) } else { 0 };
    let rows = log_rows(&app.logs, columns);
    egui::ScrollArea::both()
        .id_salt("drawer_logs")
        .auto_shrink([false, false])
        .stick_to_bottom(app.log_follow)
        .show_rows(ui, LOG_ROW_H, rows.len(), |ui, range| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            for (text, failed) in &rows[range.start..range.end] {
                ui.label(
                    RichText::new(text.as_ref())
                        .monospace()
                        .size(11.5)
                        .color(if *failed {
                            theme::BAD
                        } else {
                            theme::FG_SECONDARY
                        }),
                );
            }
        });

    if let Some(name) = switch_to {
        app.select_log_container(name);
    }
    if let Some(on) = restamped {
        app.set_log_timestamps(on);
    }
}

/// One log row's height. Uniform, which is what the pane's virtualization needs.
const LOG_ROW_H: f32 = 16.0;

/// How many monospace characters fit the log pane, for wrapping. Log lines are
/// monospace, so counting characters is the same as measuring the text — and
/// cheaper than laying every line out.
fn log_columns(ui: &Ui) -> usize {
    let font = egui::FontId::monospace(11.5);
    // the scrollbar and the pane's own padding sit inside the available width
    let width = (ui.available_width() - 16.0).max(40.0);
    let advance = ui.ctx().fonts_mut(|fonts| fonts.glyph_width(&font, ' '));
    if advance <= 0.0 {
        return 0;
    }
    ((width / advance).floor() as usize).max(20)
}

/// The rows the log pane draws: one per line when wrapping is off, and one per
/// chunk of a line when it is on. A row carries the failure flag of the line it
/// came from, so a red line stays red across all of its rows.
fn log_rows(lines: &[String], columns: usize) -> Vec<(std::borrow::Cow<'_, str>, bool)> {
    use std::borrow::Cow;
    // annotated: `Cow` cannot infer its borrowed type from one variant alone,
    // and every row here borrows from the line it came from
    let mut rows: Vec<(Cow<'_, str>, bool)> = Vec::with_capacity(lines.len());
    for line in lines {
        let text = notice::strip(line);
        let failed = notice::is_failure(line);
        if columns == 0 || text.chars().count() <= columns {
            rows.push((Cow::Borrowed(text), failed));
            continue;
        }
        let mut rest = text;
        while !rest.is_empty() {
            let take = rest
                .char_indices()
                .nth(columns)
                .map(|(at, _)| at)
                .unwrap_or(rest.len());
            rows.push((Cow::Borrowed(&rest[..take]), failed));
            rest = &rest[take..];
        }
    }
    rows
}

// ---------------------------------------------------------------------------
// Conditions
// ---------------------------------------------------------------------------

struct Condition {
    type_: String,
    status: String,
    reason: String,
    message: String,
    last_transition: Option<i64>,
}

fn conditions_of(object: &kube::core::DynamicObject) -> Vec<Condition> {
    object
        .data
        .get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|c| Condition {
                    type_: c
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    status: c
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    reason: c
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    message: c
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    last_transition: c
                        .get("lastTransitionTime")
                        .and_then(Value::as_str)
                        .and_then(parse_secs),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn conditions_tab(ui: &mut Ui, conditions: &[Condition]) {
    egui::ScrollArea::vertical()
        .id_salt("drawer_conditions")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(10.0);
            for c in conditions {
                let (color, label) = match c.status.as_str() {
                    "True" => (theme::OK, "True"),
                    "False" => (theme::BAD, "False"),
                    other => (theme::WARN, other),
                };
                let width = ui.available_width() - 24.0;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::hover());
                ui.painter().text(
                    egui::pos2(rect.left() + 12.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    &c.type_,
                    FontId::proportional(12.5),
                    theme::FG,
                );
                let time = c
                    .last_transition
                    .map(|sec| format!("{} ago", human_age(sec)))
                    .unwrap_or_default();
                ui.painter().text(
                    egui::pos2(rect.right() - 12.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    time,
                    FontId::proportional(11.0),
                    theme::FG_MUTED,
                );
                ui.painter().text(
                    egui::pos2(rect.left() + 12.0, rect.bottom() + 12.0),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(11.5),
                    color,
                );
                if !c.reason.is_empty() {
                    ui.painter().text(
                        egui::pos2(rect.left() + 70.0, rect.bottom() + 12.0),
                        Align2::LEFT_CENTER,
                        &c.reason,
                        FontId::proportional(11.5),
                        theme::FG_MUTED,
                    );
                }
                ui.add_space(34.0);
                if !c.message.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(&c.message)
                                .size(11.5)
                                .color(theme::FG_SECONDARY),
                        );
                    });
                    ui.add_space(10.0);
                }
            }
            ui.add_space(16.0);
        });
}

// ---------------------------------------------------------------------------
// YAML
// ---------------------------------------------------------------------------

fn yaml_tab(ui: &mut Ui, app: &mut App) {
    // cloned: the editor mutates `app`, and the tab stays simple to read
    match app.yaml.clone() {
        None => {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
            });
        }
        Some(Ok(text)) => {
            let editing = app.yaml_draft.is_some();
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!("{} lines", text.lines().count()))
                        .size(11.0)
                        .color(theme::FG_MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    if editing {
                        if app.yaml_changed()
                            && widgets::primary_button(ui, "Review & Apply").clicked()
                        {
                            app.yaml_review = true;
                        }
                        ui.add_space(6.0);
                        if widgets::secondary_button(ui, '\0', "Cancel").clicked() {
                            app.cancel_yaml_edit();
                        }
                    } else if widgets::secondary_button(ui, '\0', "Edit YAML").clicked() {
                        app.start_yaml_edit();
                    }
                    ui.add_space(6.0);
                    if widgets::secondary_button(ui, '\0', "Copy").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                });
            });

            match &app.yaml_draft {
                Some(draft) => {
                    let mut draft = draft.clone();
                    egui::ScrollArea::both()
                        .id_salt("drawer_yaml_editor")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let response = ui.add(
                                egui::TextEdit::multiline(&mut draft)
                                    .font(egui::TextStyle::Monospace)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(30),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::TextEdit,
                                    true,
                                    "YAML draft",
                                )
                            });
                        });
                    app.yaml_draft = Some(draft);
                }
                None => {
                    let text = text.clone();
                    egui::ScrollArea::both()
                        .id_salt("drawer_yaml")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(text)
                                    .monospace()
                                    .size(11.5)
                                    .color(theme::FG_SECONDARY),
                            );
                        });
                }
            }
        }
        Some(Err(err)) => {
            ui.add_space(16.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!("Failed to load YAML: {err}"))
                        .size(12.0)
                        .color(theme::BAD),
                );
            });
        }
    }
}
