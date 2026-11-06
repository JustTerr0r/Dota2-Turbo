//! Runs the Minify engine sidecar and turns its JSON-lines output into app events.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::{Command, CommandEvent};
use tauri_plugin_shell::ShellExt;

pub const EVENT_LOG: &str = "engine://event";
pub const EVENT_DONE: &str = "engine://done";

/// Only one engine task (patch/uninstall) may run at a time.
#[derive(Default)]
pub struct EngineState {
    busy: AtomicBool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EngineStatus {
    pub version: String,
    pub steam_root: String,
    pub steam_library: String,
    pub dota_found: bool,
    pub dota_running: bool,
    pub workshop_installed: bool,
    pub output_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Task {
    Patch,
    Uninstall,
}

impl Task {
    fn arg(self) -> &'static str {
        match self {
            Task::Patch => "patch",
            Task::Uninstall => "uninstall",
        }
    }

    /// Localization keys the engine emits when the task finished well / badly.
    fn success_key(self) -> &'static str {
        match self {
            Task::Patch => "success_terminal",
            Task::Uninstall => "mods_removed_terminal",
        }
    }
}

const FAILURE_KEYS: &[&str] = &["failure_terminal", "conflicts_detected"];

#[cfg(target_os = "macos")]
fn engine_path_env() -> String {
    // Finder-launched apps get a bare PATH; keep wine and the usual tool dirs reachable
    let mut dirs: Vec<String> = Vec::new();
    if let Some(wine) = crate::deps::wine_bin_dir() {
        dirs.push(wine.display().to_string());
    }
    dirs.extend(
        [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        .map(String::from),
    );
    if let Ok(path) = std::env::var("PATH") {
        dirs.push(path);
    }
    dirs.join(":")
}

fn engine_env(home: &Path) -> Vec<(&'static str, String)> {
    #[allow(unused_mut)]
    let mut env = vec![
        ("MINIFY_HOME", home.display().to_string()),
        ("MINIFY_EVENTS", "json".into()),
        ("MINIFY_NONINTERACTIVE", "1".into()),
        ("PYTHONUNBUFFERED", "1".into()),
        // Output is read as UTF-8; on Windows a piped stdout would use the ANSI code page
        ("PYTHONIOENCODING", "utf-8".into()),
    ];
    #[cfg(target_os = "macos")]
    env.extend([("WINEDEBUG", "-all".into()), ("PATH", engine_path_env())]);
    env
}

/// Sidecar path relative to the app exe. The Windows portable build keeps the onedir engine
/// (`d2turbo-engine.exe` + `_internal/`) in `engine/`, so `D2Turbo.exe` is the only exe users see.
#[cfg(windows)]
const SIDECAR: &str = "engine/d2turbo-engine";
#[cfg(not(windows))]
const SIDECAR: &str = "d2turbo-engine";

fn command(app: &AppHandle, home: &Path, arg: &str) -> Result<Command, String> {
    Ok(app
        .shell()
        .sidecar(SIDECAR)
        .map_err(|e| format!("engine sidecar: {e}"))?
        .arg(arg)
        .current_dir(home)
        .envs(engine_env(home)))
}

/// Parses one stdout line: engine JSON events pass through, anything else becomes a raw log line.
pub fn parse_line(line: &str) -> Option<Value> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.trim().is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(line) {
        Ok(v) if v.get("type").is_some() => Some(v),
        _ => Some(json!({ "type": "log", "key": null, "level": "info", "text": strip_ansi(line) })),
    }
}

/// Mirrors an engine event into the app log (download progress only on finish/error).
fn log_event(task: Task, v: &Value) {
    let text = v["text"].as_str().unwrap_or_default();
    match v["type"].as_str() {
        Some("log") => {
            let level = match v["level"].as_str() {
                Some("error") => log::Level::Error,
                Some("warning") => log::Level::Warn,
                _ => log::Level::Info,
            };
            match v["key"].as_str() {
                Some(key) => log::log!(target: "engine", level, "[{task:?}] {text} ({key})"),
                None => log::log!(target: "engine", level, "[{task:?}] {text}"),
            }
        }
        Some("download") if v["status"] != "downloading" => {
            log::info!(target: "engine", "[{task:?}] download {v}")
        }
        _ => {}
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub async fn status(app: &AppHandle, home: &Path) -> Result<EngineStatus, String> {
    let out = command(app, home, "status")?
        .output()
        .await
        .map_err(|e| format!("engine status: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        log::warn!(target: "engine", "status stderr: {}", stderr.trim());
    }
    stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v["type"] == "status")
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or_else(|| {
            format!(
                "engine status failed (exit {:?}): {}{}",
                out.status.code(),
                stdout.trim(),
                stderr.trim()
            )
        })
}

/// Starts a task in the background; progress arrives as `engine://event`, the result as `engine://done`.
pub fn start(app: AppHandle, state: &EngineState, home: &Path, task: Task) -> Result<(), String> {
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Another task is already running".into());
    }
    let spawned = command(&app, home, task.arg())
        .and_then(|c| c.spawn().map_err(|e| format!("start engine: {e}")));
    let (mut rx, child) = match spawned {
        Ok(v) => v,
        Err(e) => {
            log::error!("start {task:?}: {e}");
            state.busy.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };

    tauri::async_runtime::spawn(async move {
        let _child = child; // keep the handle alive for the whole run
        let mut outcome: Option<bool> = None;
        let mut code = None;
        let mut buf = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(pos) = buf.find('\n') {
                        let line: String = buf.drain(..=pos).collect();
                        if let Some(v) = parse_line(&line) {
                            match v["key"].as_str() {
                                Some(k) if k == task.success_key() => {
                                    outcome = outcome.or(Some(true))
                                }
                                Some(k) if FAILURE_KEYS.contains(&k) => outcome = Some(false),
                                _ => {}
                            }
                            log_event(task, &v);
                            let _ = app.emit(EVENT_LOG, v);
                        }
                    }
                }
                CommandEvent::Error(e) => {
                    log::error!(target: "engine", "[{task:?}] {e}");
                    let _ = app.emit(
                        EVENT_LOG,
                        json!({ "type": "log", "key": null, "level": "error", "text": e }),
                    );
                }
                CommandEvent::Terminated(payload) => code = payload.code,
                _ => {}
            }
        }
        if let Some(v) = parse_line(&buf) {
            log_event(task, &v);
            let _ = app.emit(EVENT_LOG, v);
        }
        let success = outcome.unwrap_or(false) && code == Some(0);
        log::info!("task {task:?} finished: success={success} exit={code:?} outcome={outcome:?}");
        app.state::<EngineState>()
            .busy
            .store(false, Ordering::SeqCst);
        let _ = app.emit(
            EVENT_DONE,
            json!({ "task": task, "success": success, "code": code }),
        );
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_engine_events_through() {
        let v = parse_line(
            r#"{"type": "log", "key": "success_terminal", "level": "success", "text": "ok"}"#,
        )
        .unwrap();
        assert_eq!(v["key"], "success_terminal");
    }

    #[test]
    fn wraps_plain_output_and_strips_colors() {
        let v = parse_line("\u{1b}[38;2;255;0;0mboom\u{1b}[0m\n").unwrap();
        assert_eq!(v["type"], "log");
        assert_eq!(v["text"], "boom");
        assert!(parse_line("   \n").is_none());
    }

    #[test]
    fn json_without_type_is_raw() {
        let v = parse_line("[1, 2]").unwrap();
        assert_eq!(v["text"], "[1, 2]");
    }
}
