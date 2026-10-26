//! Design tokens copied from the Vue app (`frontend/src/style.css`) and
//! wired into egui's style, so both surfaces share one visual language.

use eframe::egui;
use egui::{Color32, Stroke};

// -- Backgrounds (`@theme` in style.css) --
pub const SURFACE: Color32 = Color32::from_rgb(0x0b, 0x0f, 0x14);
pub const SURFACE_SECONDARY: Color32 = Color32::from_rgb(0x11, 0x18, 0x21);
pub const SURFACE_TERTIARY: Color32 = Color32::from_rgb(0x1a, 0x24, 0x31);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(0x25, 0x32, 0x43);

// -- Edges: edge = fg@0.16 over surface, computed once --
pub const EDGE: Color32 = Color32::from_rgb(0x29, 0x2e, 0x34);

// -- Text --
pub const FG: Color32 = Color32::from_rgb(0xe8, 0xed, 0xf4);
pub const FG_SECONDARY: Color32 = Color32::from_rgb(0xc7, 0xd0, 0xdb);
pub const FG_MUTED: Color32 = Color32::from_rgb(0x79, 0x82, 0x8c); // fg-muted 58% over surface

// -- Accent (warm amber) --
pub const ACCENT: Color32 = Color32::from_rgb(0xf0, 0xbd, 0x63);
pub const ACCENT_MUTED: Color32 = Color32::from_rgb(0x45, 0x32, 0x11); // accent@0.25 blended

// -- Edges (hover state) + table striping --
pub const EDGE_HOVER: Color32 = Color32::from_rgb(0x3a, 0x42, 0x4c);
/// `tbody tr:nth-child(even)` sits just above the surface colour.
pub const STRIPE: Color32 = Color32::from_rgb(0x0f, 0x14, 0x1a);

/// The terminal's own background: a shade under the surface, so a shell reads
/// as a place of its own rather than another panel.
pub const TERM_BG: Color32 = Color32::from_rgb(0x07, 0x0a, 0x0e);

/// Blend towards transparent, for tints and disabled fills.
pub fn alpha(color: Color32, factor: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (color.a() as f32 * factor.clamp(0.0, 1.0)) as u8,
    )
}

// -- Status --
pub const OK: Color32 = Color32::from_rgb(0x68, 0xd3, 0x91);
pub const WARN: Color32 = Color32::from_rgb(0xf6, 0xad, 0x55);
pub const BAD: Color32 = Color32::from_rgb(0xfc, 0x81, 0x81);
pub const INFO: Color32 = Color32::from_rgb(0x63, 0xb3, 0xed);

pub fn phase_color(phase: &str) -> Color32 {
    match phase {
        "Running" => OK,
        "Pending" => WARN,
        "Succeeded" => INFO,
        "Failed" | "CrashLoopBackOff" | "Unknown" => BAD,
        _ => FG_MUTED,
    }
}

/// Apply the tokens to the egui context. Call once at startup.
pub fn install(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();

        v.panel_fill = SURFACE; // central table area
        v.window_fill = SURFACE_SECONDARY;
        v.extreme_bg_color = SURFACE_TERTIARY; // text inputs, log pane bg
        v.faint_bg_color = SURFACE_TERTIARY; // striped table rows
        v.window_stroke = Stroke::new(1.0, EDGE);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, EDGE);

        v.widgets.hovered.bg_fill = SURFACE_HOVER;
        v.widgets.active.bg_fill = ACCENT_MUTED;
        v.widgets.active.fg_stroke = Stroke::new(1.0, ACCENT);
        v.selection.bg_fill = ACCENT_MUTED;
        v.selection.stroke = Stroke::new(1.0, ACCENT);

        v.override_text_color = Some(FG);
        v.hyperlink_color = ACCENT;

        // Vue: menu-item px-3 py-1 (28px rows); real buttons get px-4 py-2 per-call
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(8.0, 6.0);
        s.button_padding = egui::vec2(12.0, 4.0);
        s.menu_margin = egui::Margin::symmetric(4, 4);
        s.menu_spacing = 4.0;
        s.interact_size.y = 28.0;
        s.icon_width_inner = 16.0;
        s.icon_width = 24.0;

        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
        ] {
            // The Vue's stylesheet carries no `border-radius` at all: its
            // buttons, selects, inputs and badges are all square. Rounded
            // corners here were a divergence, not a style.
            w.corner_radius = egui::CornerRadius::ZERO;
        }
        v.menu_corner_radius = egui::CornerRadius::ZERO;
        v.window_corner_radius = egui::CornerRadius::ZERO;
        v.widgets.noninteractive.corner_radius = egui::CornerRadius::ZERO;

        style.text_styles = [
            (egui::TextStyle::Heading, egui::FontId::proportional(16.0)),
            (egui::TextStyle::Body, egui::FontId::proportional(14.0)),
            (egui::TextStyle::Button, egui::FontId::proportional(14.0)),
            (egui::TextStyle::Small, egui::FontId::proportional(11.0)),
            (egui::TextStyle::Monospace, egui::FontId::monospace(13.0)),
        ]
        .into();
    });

    // One wheel notch should move a few rows of a table, not one. egui's native
    // default is 40 points per line — measured: a single 44-pixel row here, which
    // is what makes scrolling feel like it barely moves. A trackpad is unaffected:
    // it reports points, not lines.
    ctx.options_mut(|options| options.input_options.line_scroll_speed = 80.0);
}
