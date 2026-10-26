//! Material Design Icons (same set as the Vue app's `mdi:*`), vendored as a
//! webfont and registered as a *fallback* for the default families — so a
//! normal string can carry glyphs: `format!("{} Pods", icons::CUBE)`.

use eframe::egui;

// codepoints extracted from @mdi/font 7.x css.
// Only what the Vue app actually uses: section icons, dashboard, palette chrome.
pub const DASHBOARD: char = '\u{F0A1D}'; // mdi:view-dashboard-outline
pub const HEXAGONS: char = '\u{F10F2}'; // mdi:hexagon-multiple-outline (Workloads)
pub const MAP_MARKER: char = '\u{F07D9}'; // mdi:map-marker-outline (Discovery)
pub const COG: char = '\u{F08BB}'; // mdi:cog-outline (Configuration)
pub const HARDDISK: char = '\u{F02CA}'; // mdi:harddisk (Storage)
pub const LAN_CONNECT: char = '\u{F0318}'; // mdi:lan-connect (Network)
pub const FOLDER: char = '\u{F0256}'; // mdi:folder-outline (namespace commands)
pub const LAYERS: char = '\u{F09FE}'; // mdi:layers-outline (all namespaces)
pub const MAGNIFY: char = '\u{F0349}'; // mdi:magnify (palette search)
pub const KEYBOARD_RETURN: char = '\u{F0311}'; // mdi:keyboard-return (palette select)
pub const CLOSE: char = '\u{F0156}'; // mdi:close (drawer)
pub const CHEVRON_DOWN: char = '\u{F0140}'; // mdi:chevron-down
pub const CHECK: char = '\u{F012C}'; // mdi:check
pub const CHEVRON_RIGHT: char = '\u{F0142}'; // mdi:chevron-right
pub const ACCOUNT: char = '\u{F0009}'; // mdi:account-circle (header)

// -- page chrome / actions --
pub const CUBE_OUTLINE: char = '\u{F01A7}'; // mdi:cube-outline (drawer title)
pub const CUBE_OFF_OUTLINE: char = '\u{F141D}'; // mdi:cube-off-outline (empty state)
pub const REFRESH: char = '\u{F0450}'; // mdi:refresh (page header)
pub const MINUS: char = '\u{F0374}'; // mdi:minus (scale dialog)
pub const PLUS: char = '\u{F0415}'; // mdi:plus (scale dialog)
pub const ALERT: char = '\u{F0026}'; // mdi:alert (scale-to-zero warning)
pub const CHECK_CIRCLE_OUTLINE: char = '\u{F05E1}'; // mdi:check-circle-outline
pub const RESTART: char = '\u{F0709}'; // mdi:restart (context menu)
pub const DELETE_OUTLINE: char = '\u{F09E7}'; // mdi:delete-outline (context menu)
pub const CONSOLE: char = '\u{F018D}'; // mdi:console (drawer Logs)
pub const PENCIL_OUTLINE: char = '\u{F0CB6}'; // mdi:pencil-outline (edit a secret)
pub const CONTENT_SAVE: char = '\u{F0193}'; // mdi:content-save (save a secret)
pub const ARROW_EXPAND_VERTICAL: char = '\u{F084F}'; // mdi:arrow-expand-vertical (scale)
pub const TEXT_SEARCH: char = '\u{F13B8}'; // mdi:text-search (no search results)
pub const HOURGLASS: char = '\u{F051F}'; // mdi:timer-sand (not-ported placeholder)
pub const APPS: char = '\u{F003B}'; // mdi:apps (dashboard applications)

// -- kind icons --
pub const FOLDER_OUTLINE: char = '\u{F0256}'; // mdi:folder-outline (Namespaces)
pub const KEY_OUTLINE: char = '\u{F0DD6}'; // mdi:key-outline (Secrets)
pub const CLOCK_OUTLINE: char = '\u{F0150}'; // mdi:clock-outline (Jobs)
pub const CALENDAR_CLOCK: char = '\u{F00F0}'; // mdi:calendar-clock (CronJobs)
pub const SHIELD_OUTLINE: char = '\u{F0499}'; // mdi:shield-outline (PodDisruptionBudgets)
pub const DATABASE_OUTLINE: char = '\u{F1632}'; // mdi:database-outline (StatefulSets, PVCs)
pub const SERVER_OUTLINE: char = '\u{F1C9A}'; // mdi:server-outline (DaemonSets, Nodes)
pub const CLOUD_OUTLINE: char = '\u{F0163}'; // mdi:cloud-outline (Ingresses, context)

const FONT: &[u8] = include_bytes!("../assets/materialdesignicons-webfont.ttf");

/// Register the icon font as a fallback so icon chars can be embedded in
/// otherwise normal strings (latin text keeps rendering with the default font).
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "mdi".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(FONT)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("mdi".to_owned());
    }
    ctx.set_fonts(fonts);
}
