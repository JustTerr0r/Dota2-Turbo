//! Optional system dependencies the engine needs for the CSS/XML part of the mods.
//!
//! Iteration 1 only detects them, the UI shows install instructions for anything missing.

use serde::Serialize;

#[cfg(target_os = "macos")]
mod macos;

#[derive(Debug, Clone, Serialize)]
pub struct DepCheck {
    /// Stable id the UI maps to install instructions
    pub id: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Directory containing the `wine` binary, prepended to the engine's PATH.
/// Windows runs resourcecompiler.exe natively and needs no extra dependencies.
#[cfg(target_os = "macos")]
pub fn wine_bin_dir() -> Option<std::path::PathBuf> {
    macos::find_wine().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// Checks that don't depend on the engine (workshop tools come from the engine status).
pub fn check() -> Vec<DepCheck> {
    #[cfg(target_os = "macos")]
    return macos::check();
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}
