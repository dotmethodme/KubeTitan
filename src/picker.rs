//! The resource picker: browse every resource type the cluster serves and pin
//! the interesting ones to the sidebar.
//!
//! Mirrors Vue's `ResourcePickerDialog` ("Add Resource to Favorites"): a search
//! box, category chips, and the matching types grouped by category.

use eframe::egui::{self, Align2, FontId, RichText, Stroke, Vec2};

use crate::{App, icons, theme, widgets};

pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.picker_open {
        return;
    }

    let mut open = true;
    let mut to_add = None;

    egui::Modal::new(egui::Id::new("resource-picker"))
        .backdrop_color(egui::Color32::from_black_alpha(140))
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE_SECONDARY)
                .stroke(Stroke::new(1.0, theme::EDGE))
                .corner_radius(0.0)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(
                RichText::new("Add Resource to Favorites")
                    .size(15.0)
                    .strong()
                    .color(theme::FG),
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new("Select a resource type to add to your favorites for quick access.")
                    .size(11.5)
                    .color(theme::FG_MUTED),
            );
            ui.add_space(12.0);

            // the picker is a keyboard-first dialog: the search box keeps the
            // caret, so typing filters without clicking into it first
            app.picker_query = widgets::search_field(ui, &app.picker_query, true);
            ui.add_space(10.0);

            categories(app, ui);
            ui.add_space(8.0);

            if app.resource_types.is_empty() {
                ui.label(
                    RichText::new("Discovering resource types…")
                        .size(12.0)
                        .color(theme::FG_MUTED),
                );
            } else {
                let rows = filtered(app);
                if rows.is_empty() {
                    ui.label(
                        RichText::new(format!("No resources match \"{}\"", app.picker_query))
                            .size(12.0)
                            .color(theme::FG_MUTED),
                    );
                }
                egui::ScrollArea::vertical()
                    .id_salt("resource-picker-scroll")
                    .max_height(360.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut category = String::new();
                        for resource in &rows {
                            if resource.category != category {
                                category = resource.category.clone();
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(category.to_uppercase())
                                        .size(10.5)
                                        .color(theme::FG_MUTED),
                                );
                                ui.add_space(4.0);
                            }
                            if picker_row(ui, app, resource) {
                                to_add = Some(resource.clone());
                            }
                        }
                    });
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::secondary_button(ui, '\0', "Close").clicked() {
                    open = false;
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} favorites", app.favorites.len()))
                        .size(11.0)
                        .color(theme::FG_MUTED),
                );
            });
        });

    if let Some(resource) = to_add {
        app.add_favorite(resource);
    }
    if !open {
        app.picker_open = false;
    }
}

/// Category chips, `All` first — Vue's category filter row.
fn categories(app: &mut App, ui: &mut egui::Ui) {
    let mut names: Vec<String> = app
        .resource_types
        .iter()
        .map(|resource| resource.category.clone())
        .collect();
    names.sort();
    names.dedup();

    ui.horizontal_wrapped(|ui| {
        let selected = app.picker_category.is_none();
        if ui
            .selectable_label(selected, RichText::new("All").size(11.5))
            .clicked()
        {
            app.picker_category = None;
        }
        for name in names {
            let selected = app.picker_category.as_deref() == Some(name.as_str());
            if ui
                .selectable_label(selected, RichText::new(&name).size(11.5))
                .clicked()
            {
                app.picker_category = if selected { None } else { Some(name) };
            }
        }
    });
}

/// Search + category filter, in discovery's own order.
fn filtered(app: &App) -> Vec<crate::discovery::ResourceType> {
    let query = app.picker_query.trim().to_lowercase();
    app.resource_types
        .iter()
        .filter(|resource| match &app.picker_category {
            Some(category) => &resource.category == category,
            None => true,
        })
        .filter(|resource| {
            query.is_empty()
                || resource.kind.to_lowercase().contains(&query)
                || resource.name.to_lowercase().contains(&query)
                || resource.group.to_lowercase().contains(&query)
                || resource
                    .short_names
                    .iter()
                    .any(|short| short.to_lowercase().contains(&query))
        })
        .cloned()
        .collect()
}

/// One row: kind over API name, with an `Add` affordance on hover. Returns true
/// when the row was clicked and is not already a favorite.
fn picker_row(ui: &mut egui::Ui, app: &App, resource: &crate::discovery::ResourceType) -> bool {
    let favorite = app
        .favorites
        .iter()
        .any(|saved| saved.id() == resource.id());

    let (rect, response) = widgets::row(ui, 34.0);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 0.0, theme::SURFACE_TERTIARY);
    }
    painter.text(
        rect.left_center() + Vec2::new(10.0, -7.0),
        Align2::LEFT_CENTER,
        &resource.kind,
        FontId::proportional(12.5),
        theme::FG,
    );
    painter.text(
        rect.left_center() + Vec2::new(10.0, 8.0),
        Align2::LEFT_CENTER,
        resource.api_name(),
        FontId::proportional(10.5),
        theme::FG_MUTED,
    );
    let hint = if favorite {
        (
            format!("{} In favorites", icons::CHECK_CIRCLE_OUTLINE),
            theme::FG_MUTED,
        )
    } else if response.hovered() {
        (format!("{} Add", icons::PLUS), theme::ACCENT)
    } else {
        (String::new(), theme::FG_MUTED)
    };
    if !hint.0.is_empty() {
        painter.text(
            rect.right_center() - Vec2::new(12.0, 0.0),
            Align2::RIGHT_CENTER,
            hint.0,
            FontId::proportional(11.5),
            hint.1,
        );
    }

    response.clicked() && !favorite
}
