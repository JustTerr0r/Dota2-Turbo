use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::DepCheck;

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Apps started from Finder get a minimal PATH, so look in the usual install spots too.
pub fn find_wine() -> Option<PathBuf> {
    let h = home();
    let mut candidates: Vec<PathBuf> = env::var_os("PATH")
        .map(|p| env::split_paths(&p).map(|d| d.join("wine")).collect())
        .unwrap_or_default();
    candidates.extend([
        h.join(".local/bin/wine"),
        PathBuf::from("/opt/homebrew/bin/wine"),
        PathBuf::from("/usr/local/bin/wine"),
    ]);
    for apps in [h.join("Applications"), PathBuf::from("/Applications")] {
        for app in ["Wine Stable.app", "Wine Devel.app", "Wine Staging.app"] {
            candidates.push(apps.join(app).join("Contents/Resources/wine/bin/wine"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn wine_prefix() -> PathBuf {
    env::var_os("WINEPREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".wine"))
}

fn rosetta_installed() -> bool {
    if cfg!(not(target_arch = "aarch64")) {
        return true;
    }
    Command::new("/usr/bin/arch")
        .args(["-x86_64", "/usr/bin/true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// DXVK is installed when its DLL is in system32 and overridden to native in the prefix registry.
fn dxvk_installed(prefix: &Path) -> bool {
    let dll = prefix.join("drive_c/windows/system32/d3d11.dll");
    let overridden = fs::read_to_string(prefix.join("user.reg"))
        .map(|reg| reg.contains("\"d3d11\"=\"native"))
        .unwrap_or(false);
    dll.is_file()
        && (overridden
            || env::var("WINEDLLOVERRIDES")
                .map(|v| v.contains("d3d11"))
                .unwrap_or(false))
}

pub fn check() -> Vec<DepCheck> {
    let wine = find_wine();
    let prefix = wine_prefix();
    vec![
        DepCheck {
            id: "rosetta",
            ok: rosetta_installed(),
            detail: String::new(),
        },
        DepCheck {
            id: "wine",
            ok: wine.is_some(),
            detail: wine.map(|p| p.display().to_string()).unwrap_or_default(),
        },
        DepCheck {
            id: "dxvk",
            ok: dxvk_installed(&prefix),
            detail: prefix.display().to_string(),
        },
    ]
}
