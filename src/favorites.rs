//! Favorite resource types.
//!
//! The Wails backend keeps favorites per context; this port talks to one
//! cluster at a time, so it keeps a single list.

use crate::config;
use crate::discovery::ResourceType;

const FILE: &str = "favorites.json";

/// What the sidebar starts with, so the common types are not a hunt through the
/// picker on a fresh install.
fn defaults() -> Vec<ResourceType> {
    [
        ("pods", "Pod", "", "v1"),
        ("deployments", "Deployment", "apps", "v1"),
        ("secrets", "Secret", "", "v1"),
        ("configmaps", "ConfigMap", "", "v1"),
    ]
    .into_iter()
    .map(|(name, kind, group, version)| ResourceType {
        name: name.to_string(),
        kind: kind.to_string(),
        group: group.to_string(),
        version: version.to_string(),
        namespaced: true,
        short_names: Vec::new(),
        category: crate::discovery::categorize(group, kind).to_string(),
    })
    .collect()
}

pub fn load() -> Vec<ResourceType> {
    // A missing file is a fresh install and gets the defaults; an empty list is
    // a choice — the user removed them — and is left as it is.
    if !config::path(FILE).exists() {
        let defaults = defaults();
        let _ = save(&defaults);
        return defaults;
    }
    config::load_json(FILE)
}

pub fn save(favorites: &[ResourceType]) -> Result<(), String> {
    config::save_json(FILE, favorites)
}
