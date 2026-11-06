//! Writable engine home (cwd of the sidecar) and syncing of the bundled mods into it.
//!
//! Minify writes `mods/ config/ cache/ logs/ backup/` relative to its cwd, which can't be
//! inside the signed .app bundle, so everything lives under the app data dir.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

/// Mods shipped inside the app, everything else from Minify is left out of the bundle.
pub const BUNDLED_MODS: &[&str] = &["#base", "#English Fix", "D2Turbo", "OpenDotaGuides Guides"];

const MARKER: &str = ".bundle-version";

pub fn engine_home(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?
        .join("engine");
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    Ok(dir)
}

pub fn bundle_id() -> String {
    format!("{}+{}", env!("CARGO_PKG_VERSION"), env!("GIT_SHA"))
}

/// Replaces the bundled mod folders in `home/mods` when the app build changed.
/// Debug builds always resync so mod edits show up on the next launch.
pub fn sync_mods(app: &AppHandle, home: &Path) -> Result<(), String> {
    let src_root = app
        .path()
        .resource_dir()
        .map_err(|e| format!("resource dir: {e}"))?
        .join("mods");
    sync_mods_from(
        &src_root,
        &home.join("mods"),
        &bundle_id(),
        cfg!(debug_assertions),
    )
}

pub fn sync_mods_from(
    src_root: &Path,
    dst_root: &Path,
    id: &str,
    force: bool,
) -> Result<(), String> {
    let marker = dst_root.join(MARKER);
    if !force
        && fs::read_to_string(&marker)
            .map(|s| s == id)
            .unwrap_or(false)
    {
        return Ok(());
    }
    fs::create_dir_all(dst_root).map_err(|e| e.to_string())?;
    for name in BUNDLED_MODS {
        let src = src_root.join(name);
        if !src.is_dir() {
            return Err(format!("bundled mod missing: {}", src.display()));
        }
        let dst = dst_root.join(name);
        if dst.exists() {
            fs::remove_dir_all(&dst).map_err(|e| format!("remove {}: {e}", dst.display()))?;
        }
        copy_dir(&src, &dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
    }
    fs::write(&marker, id).map_err(|e| e.to_string())
}

pub fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "__pycache__" || name == ".DS_Store" {
            continue;
        }
        let target = dst.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bundle(root: &Path) {
        for name in BUNDLED_MODS {
            fs::create_dir_all(root.join(name).join("__pycache__")).unwrap();
            fs::write(root.join(name).join("manifest.json"), "{}").unwrap();
        }
    }

    #[test]
    fn syncs_once_per_bundle_id_and_skips_pycache() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        make_bundle(&src);

        sync_mods_from(&src, &dst, "1", false).unwrap();
        assert!(dst.join("D2Turbo/manifest.json").exists());
        assert!(!dst.join("D2Turbo/__pycache__").exists());

        // same id: user-side changes are kept
        fs::write(dst.join("D2Turbo/extra"), "x").unwrap();
        sync_mods_from(&src, &dst, "1", false).unwrap();
        assert!(dst.join("D2Turbo/extra").exists());

        // new id: folders are replaced
        sync_mods_from(&src, &dst, "2", false).unwrap();
        assert!(!dst.join("D2Turbo/extra").exists());
    }

    #[test]
    fn missing_bundled_mod_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            sync_mods_from(&tmp.path().join("nope"), &tmp.path().join("dst"), "1", true).is_err()
        );
    }
}
