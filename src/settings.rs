//! Preferences that outlive the process.
//!
//! Vue keeps these in `localStorage` through its Pinia persistence plugin; the
//! port writes them next to the favorites. Unknown or missing fields fall back
//! to their defaults, so an older file stays readable after a field is added.

use crate::config;

const FILE: &str = "settings.json";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Namespace the views are filtered to; empty means all namespaces.
    pub namespace: String,
    /// Kubeconfig context to open with. `None` keeps kube's own default, which
    /// is the in-cluster service account when there is no kubeconfig.
    pub context: Option<String>,
    /// Width the resource drawer was dragged to. `None` means it was never
    /// dragged, so the default applies and a narrower window can still shrink it.
    pub drawer_width: Option<f32>,
}

pub fn load() -> Settings {
    config::load_json(FILE)
}

pub fn save(settings: &Settings) -> Result<(), String> {
    config::save_json(FILE, settings)
}
