//! The fixed D2Turbo mod set, written to the engine's `config/mods.json` before every run.

use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

/// Mods enabled by the Patch button (`#base` is always applied by the engine itself).
pub const ENABLED_MODS: &[&str] = &["#English Fix", "D2Turbo", "OpenDotaGuides Guides"];

pub fn write_mods_config(home: &Path) -> Result<(), String> {
    let mods_dir = home.join("mods");
    let mut states = Map::new();
    let entries =
        fs::read_dir(&mods_dir).map_err(|e| format!("read {}: {e}", mods_dir.display()))?;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        states.insert(
            name.clone(),
            Value::Bool(ENABLED_MODS.contains(&name.as_str())),
        );
    }
    let config_dir = home.join("config");
    fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    // serde_json::Map is sorted, matching how the engine writes the file
    let json = serde_json::to_string_pretty(&Value::Object(states)).map_err(|e| e.to_string())?;
    fs::write(config_dir.join("mods.json"), json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_only_preset_mods() {
        let tmp = tempfile::tempdir().unwrap();
        for name in [
            "#base",
            "#English Fix",
            "D2Turbo",
            "OpenDotaGuides Guides",
            "Tree Mod",
        ] {
            fs::create_dir_all(tmp.path().join("mods").join(name)).unwrap();
        }
        fs::write(tmp.path().join("mods/.bundle-version"), "x").unwrap();

        write_mods_config(tmp.path()).unwrap();

        let raw = fs::read_to_string(tmp.path().join("config/mods.json")).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["D2Turbo"], true);
        assert_eq!(v["#English Fix"], true);
        assert_eq!(v["OpenDotaGuides Guides"], true);
        assert_eq!(v["#base"], false);
        assert_eq!(v["Tree Mod"], false);
        assert!(v.get(".bundle-version").is_none());
    }
}
