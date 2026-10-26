//! KubeTitan-RS — native egui + kube-rs port of the KubeTitan desktop app.
//!
//! Module map:
//!   `kinds.rs`   resource catalog — the single source of truth for views
//!   `cluster.rs` informers and write operations against the cluster
//!   `widgets.rs` reusable UI primitives (table, header, dialogs)
//!   `views.rs`   central content (dashboard + generic resource list)
//!   `drawer.rs`  right-hand resource drawer
//!   `ui.rs`      chrome: header bar and sidebar
//!   `palette.rs` Ctrl+K command palette
//!   `theme.rs`, `icons.rs` design tokens and mdi glyphs
//!
//! The eframe thread only touches in-memory state; everything that talks to
//! the cluster runs on a tokio runtime and reports back over an mpsc channel.

mod app;
mod cluster;
mod config;
mod control;
mod discovery;
mod drawer;
mod favorites;
mod icons;
mod kinds;
mod notice;
mod palette;
mod picker;
mod portforward;
mod prometheus;
mod settings;
mod terminal;
mod theme;
mod ui;
mod views;
mod widgets;

pub use app::{App, DrawerTab, Msg, PaletteMode, Pending, Selection, View};

use anyhow::Result;
use eframe::egui;
use std::sync::mpsc;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let shot_path = args
        .iter()
        .position(|a| a == "--shot")
        .and_then(|i| args.get(i + 1).cloned());
    let open_palette = args.iter().any(|a| a == "--palette");
    // Agent control channel: drive the app over stdin/stdout instead of a mouse.
    let control = args.iter().any(|a| a == "--control");

    let rt = tokio::runtime::Runtime::new()?;
    // Reopen on the context picked last run; if it has gone away (kubeconfig
    // edited, cluster deleted) fall back to kube's own resolution and forget it.
    let mut settings = settings::load();
    let client = match rt.block_on(cluster::connect(settings.context.as_deref())) {
        Ok(client) => client,
        Err(e) => {
            eprintln!(
                "context `{}` unavailable: {e}",
                settings.context.as_deref().unwrap_or("default")
            );
            settings.context = None;
            let _ = settings::save(&settings);
            rt.block_on(cluster::connect_default())
                .map_err(anyhow::Error::msg)?
        }
    };
    let (tx, rx) = mpsc::channel();

    if control {
        control::spawn_stdio(tx.clone());
    }

    // `KT_WINDOW=1600x1000` opens a bigger window. The default is the size the
    // app is designed at; the override exists for screenshots and for driving
    // the UI on a larger virtual screen.
    let (width, height) = std::env::var("KT_WINDOW")
        .ok()
        .and_then(|spec| {
            let (w, h) = spec.split_once('x')?;
            Some((w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?))
        })
        .unwrap_or((1280.0, 820.0));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([width, height]),
        ..Default::default()
    };
    eframe::run_native(
        "KubeTitan-RS",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            icons::install(&cc.egui_ctx);
            Ok(Box::new(App::new(
                rt,
                client,
                tx,
                rx,
                cc.egui_ctx.clone(),
                shot_path,
                open_palette,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}
