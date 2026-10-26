//! Where the port keeps its state on disk.
//!
//! The Wails app stores preferences through its own app config; the port keeps
//! plain JSON under `$XDG_CONFIG_HOME/kubetitan/` (usually `~/.config/...`),
//! the same directory on every platform it runs on.

use std::path::PathBuf;

/// Directory holding the port's JSON state.
pub fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("kubetitan")
}

/// Path of one state file inside [`dir`].
pub fn path(file: &str) -> PathBuf {
    dir().join(file)
}

/// Read a JSON state file, falling back to the default when it is missing,
/// unreadable or corrupt — a broken file means "no state yet", not a crash.
pub fn load_json<T>(file: &str) -> T
where
    T: serde::de::DeserializeOwned + Default,
{
    let path = path(file);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return T::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        crate::cluster::debug_log(&format!("{file} ignored ({}): {e}", path.display()));
        T::default()
    })
}

/// Write a JSON state file, creating the directory on first use.
pub fn save_json<T>(file: &str, value: &T) -> Result<(), String>
where
    T: serde::Serialize + ?Sized,
{
    let path = path(file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
}
