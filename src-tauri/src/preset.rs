//! The fixed D2Turbo mod set, written to the engine's `config/mods.json` before every run,
//! and the launcher's feature switches.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Mods enabled by the Patch button (`#base` is always applied by the engine itself).
pub const ENABLED_MODS: &[&str] = &["#English Fix", "D2Turbo"];
/// Enabled on top of `ENABLED_MODS` only with `Features::turbo_builds`.
const GUIDES_MOD: &str = "OpenDotaGuides Guides";

const MOD_NAME: &str = "D2Turbo";

/// Optional parts of D2Turbo, the league rating is always on.
/// Stored as settings of the mod (see its `manifest.json`), which reads them while patching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Features {
    pub turbo_builds: bool,
    pub turbo_picks: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            turbo_builds: true,
            turbo_picks: true,
        }
    }
}

fn config_path(home: &Path) -> std::path::PathBuf {
    home.join("config").join("minify_config.json")
}

pub fn read_features(home: &Path) -> Features {
    let default = Features::default();
    let config = fs::read_to_string(config_path(home))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let Some(config) = config else {
        return default;
    };
    let mod_conf = &config["modconf"][MOD_NAME];
    Features {
        turbo_builds: mod_conf["turbo_builds"]
            .as_bool()
            .unwrap_or(default.turbo_builds),
        turbo_picks: mod_conf["turbo_picks"]
            .as_bool()
            .unwrap_or(default.turbo_picks),
    }
}

/// Merges the switches into the engine config, keeping everything else in the file.
pub fn write_features(home: &Path, features: Features) -> Result<(), String> {
    let path = config_path(home);
    let mut config = match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str::<Value>(&raw)
            .map_err(|e| format!("parse {}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };
    let root = config
        .as_object_mut()
        .ok_or_else(|| format!("{} is not an object", path.display()))?;
    let modconf = root.entry("modconf").or_insert_with(|| json!({}));
    if !modconf.is_object() {
        *modconf = json!({});
    }
    let mod_conf = modconf
        .as_object_mut()
        .expect("modconf is an object")
        .entry(MOD_NAME)
        .or_insert_with(|| json!({}));
    if !mod_conf.is_object() {
        *mod_conf = json!({});
    }
    mod_conf["turbo_builds"] = json!(features.turbo_builds);
    mod_conf["turbo_picks"] = json!(features.turbo_picks);

    fs::create_dir_all(home.join("config")).map_err(|e| e.to_string())?;
    let out = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&path, out).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn write_mods_config(home: &Path, features: Features) -> Result<(), String> {
    let mods_dir = home.join("mods");
    let mut states = Map::new();
    let entries =
        fs::read_dir(&mods_dir).map_err(|e| format!("read {}: {e}", mods_dir.display()))?;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let enabled =
            ENABLED_MODS.contains(&name.as_str()) || (name == GUIDES_MOD && features.turbo_builds);
        states.insert(name, Value::Bool(enabled));
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

        write_mods_config(tmp.path(), Features::default()).unwrap();

        let raw = fs::read_to_string(tmp.path().join("config/mods.json")).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["D2Turbo"], true);
        assert_eq!(v["#English Fix"], true);
        assert_eq!(v["OpenDotaGuides Guides"], true);
        assert_eq!(v["#base"], false);
        assert_eq!(v["Tree Mod"], false);
        assert!(v.get(".bundle-version").is_none());
    }

    #[test]
    fn turbo_builds_off_disables_only_the_guides_mod() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["#English Fix", "D2Turbo", "OpenDotaGuides Guides"] {
            fs::create_dir_all(tmp.path().join("mods").join(name)).unwrap();
        }
        let features = Features {
            turbo_builds: false,
            turbo_picks: true,
        };

        write_mods_config(tmp.path(), features).unwrap();

        let raw = fs::read_to_string(tmp.path().join("config/mods.json")).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["OpenDotaGuides Guides"], false);
        assert_eq!(v["D2Turbo"], true);
        assert_eq!(v["#English Fix"], true);
    }

    #[test]
    fn features_default_on_when_not_configured() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_features(tmp.path()), Features::default());

        fs::create_dir_all(tmp.path().join("config")).unwrap();
        fs::write(config_path(tmp.path()), "not json").unwrap();
        assert_eq!(read_features(tmp.path()), Features::default());
    }

    #[test]
    fn write_features_keeps_other_settings() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("config")).unwrap();
        fs::write(
            config_path(tmp.path()),
            r#"{"steam_id": "1", "modconf": {"D2Turbo": {"guide_title": "X"}, "Other": {"a": 1}}}"#,
        )
        .unwrap();
        let features = Features {
            turbo_builds: false,
            turbo_picks: false,
        };

        write_features(tmp.path(), features).unwrap();

        assert_eq!(read_features(tmp.path()), features);
        let raw = fs::read_to_string(config_path(tmp.path())).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["steam_id"], "1");
        assert_eq!(v["modconf"]["D2Turbo"]["guide_title"], "X");
        assert_eq!(v["modconf"]["Other"]["a"], 1);
    }

    #[test]
    fn write_features_refuses_to_overwrite_a_broken_config() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("config")).unwrap();
        fs::write(config_path(tmp.path()), "not json").unwrap();

        assert!(write_features(tmp.path(), Features::default()).is_err());
        assert_eq!(
            fs::read_to_string(config_path(tmp.path())).unwrap(),
            "not json"
        );
    }
}
