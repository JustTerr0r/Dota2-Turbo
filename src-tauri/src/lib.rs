mod deps;
mod engine;
mod preset;
mod report;
mod workspace;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
use tauri_plugin_opener::OpenerExt;

use engine::{EngineState, EngineStatus, Task};

#[derive(Serialize)]
struct AppStatus {
    app_version: &'static str,
    build: &'static str,
    /// `macos` / `windows`, picks the platform-specific install instructions in the UI
    platform: &'static str,
    engine: EngineStatus,
    deps: Vec<deps::DepCheck>,
    engine_home: String,
}

/// Last `get_status` result (or its error), attached to bug reports.
#[derive(Default)]
struct LastStatus(Mutex<Option<Value>>);

/// Prepares the engine home (bundled mods + preset) so the engine always sees the D2Turbo set.
fn prepare(app: &AppHandle) -> Result<PathBuf, String> {
    let home = workspace::engine_home(app)?;
    workspace::sync_mods(app, &home)?;
    preset::write_mods_config(&home)?;
    Ok(home)
}

#[tauri::command]
async fn get_status(app: AppHandle, last: State<'_, LastStatus>) -> Result<AppStatus, String> {
    let result = collect_status(&app).await;
    let snapshot = match &result {
        Ok(s) => {
            let v = serde_json::to_value(s).unwrap_or_default();
            log::info!("status: {v}");
            v
        }
        Err(e) => {
            log::error!("status failed: {e}");
            json!({ "error": e })
        }
    };
    *last.0.lock().unwrap() = Some(snapshot);
    result
}

async fn collect_status(app: &AppHandle) -> Result<AppStatus, String> {
    let home = prepare(app)?;
    let engine = engine::status(app, &home).await?;
    let deps = tauri::async_runtime::spawn_blocking(deps::check)
        .await
        .map_err(|e| e.to_string())?;
    Ok(AppStatus {
        app_version: env!("CARGO_PKG_VERSION"),
        build: env!("GIT_SHA"),
        platform: std::env::consts::OS,
        engine,
        deps,
        engine_home: home.display().to_string(),
    })
}

#[tauri::command]
fn run_task(app: AppHandle, state: State<'_, EngineState>, task: Task) -> Result<(), String> {
    log::info!("run_task {task:?}");
    let result = prepare(&app).and_then(|home| engine::start(app.clone(), &state, &home, task));
    if let Err(e) = &result {
        log::error!("run_task {task:?} failed to start: {e}");
    }
    result
}

const RESCOMP_EXE: &str = "game/bin/win64/resourcecompiler.exe";

/// Accepts either a `rescomproot` folder or a folder that contains one.
fn find_rescomp_root(picked: &Path) -> Option<PathBuf> {
    [picked.to_path_buf(), picked.join("rescomproot")]
        .into_iter()
        .find(|p| p.join(RESCOMP_EXE).is_file())
}

#[tauri::command]
async fn import_workshop_tools(app: AppHandle, path: String) -> Result<(), String> {
    log::info!("import_workshop_tools from {path}");
    let result = import_workshop_tools_inner(app, path).await;
    if let Err(e) = &result {
        log::error!("import_workshop_tools failed: {e}");
    }
    result
}

async fn import_workshop_tools_inner(app: AppHandle, path: String) -> Result<(), String> {
    let src = find_rescomp_root(Path::new(&path))
        .ok_or_else(|| format!("{RESCOMP_EXE} not found in the selected folder"))?;
    let dst = workspace::engine_home(&app)?
        .join("config")
        .join("rescomproot");
    tauri::async_runtime::spawn_blocking(move || {
        let tmp = dst.with_extension("importing");
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp).map_err(|e| e.to_string())?;
        }
        workspace::copy_dir(&src, &tmp).map_err(|e| format!("copy workshop tools: {e}"))?;
        if dst.exists() {
            std::fs::remove_dir_all(&dst).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn open_logs(app: AppHandle) -> Result<(), String> {
    let logs = workspace::engine_home(&app)?.join("logs");
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(logs.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Context from the UI that only it knows about.
#[derive(Deserialize)]
struct ReportRequest {
    comment: String,
    contact: String,
    /// Last task result shown on screen, e.g. `{"task": "patch", "success": false}`
    last_result: Option<Value>,
}

#[tauri::command]
async fn send_report(
    app: AppHandle,
    last: State<'_, LastStatus>,
    request: ReportRequest,
) -> Result<report::ReportResult, String> {
    let id = report::new_id();
    log::info!("report {id}: building");
    let status = last.0.lock().unwrap().clone();
    let meta = json!({
        "id": id,
        "created_at": report::unix_now(),
        "app_version": env!("CARGO_PKG_VERSION"),
        "build": env!("GIT_SHA"),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "os_version": report::os_version(),
        "comment": request.comment.trim(),
        "contact": request.contact.trim(),
        "last_result": request.last_result,
        "status": status,
    });
    let app_logs = app.path().app_log_dir().map_err(|e| e.to_string())?;
    let console_log = status
        .as_ref()
        .and_then(|s| s["engine"]["steam_library"].as_str())
        .filter(|lib| !lib.is_empty())
        .map(|lib| PathBuf::from(lib).join("steamapps/common/dota 2 beta/game/dota/console.log"));
    let src = report::Sources {
        engine_home: workspace::engine_home(&app)?,
        console_log,
        user_home: app.path().home_dir().ok().map(|p| p.display().to_string()),
        app_logs: app_logs.clone(),
    };
    let reports = app_logs.join("reports");
    let zip_path = reports.join(format!("{id}.zip"));
    {
        let (meta, zip_path) = (meta.clone(), zip_path.clone());
        tauri::async_runtime::spawn_blocking(move || {
            let r = report::build_zip(&src, &meta, &zip_path);
            report::prune_reports(&reports);
            r
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    let error = report::upload(&id, &meta, &zip_path).await.err();
    match &error {
        None => log::info!("report {id}: sent to {}", report::REPORT_URL),
        Some(e) => log::error!("report {id}: upload failed: {e}"),
    }
    Ok(report::ReportResult {
        id,
        sent: error.is_none(),
        error,
        path: zip_path.display().to_string(),
    })
}

/// Shows a saved report in Finder / Explorer.
#[tauri::command]
fn reveal_report(app: AppHandle, path: String) -> Result<(), String> {
    let reports = app
        .path()
        .app_log_dir()
        .map_err(|e| e.to_string())?
        .join("reports");
    let path = PathBuf::from(path);
    if !path.starts_with(&reports) {
        return Err("not a report file".into());
    }
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| e.to_string())
}

/// UI errors and notable actions, so they end up in the same log file.
#[tauri::command]
fn log_ui(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!(target: "ui", "{message}"),
        "warn" => log::warn!(target: "ui", "{message}"),
        _ => log::info!(target: "ui", "{message}"),
    }
}

fn logger() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let mut targets = vec![Target::new(TargetKind::LogDir {
        file_name: Some("D2Turbo".into()),
    })];
    if cfg!(debug_assertions) {
        targets.push(Target::new(TargetKind::Stdout));
    }
    tauri_plugin_log::Builder::new()
        .clear_targets()
        .targets(targets)
        .level(log::LevelFilter::Info)
        .max_file_size(5 * 1024 * 1024)
        .rotation_strategy(RotationStrategy::KeepSome(3))
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default_hook(info);
    }));

    tauri::Builder::default()
        .plugin(logger())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(EngineState::default())
        .manage(LastStatus::default())
        .setup(|app| {
            log::info!(
                "D2Turbo {} ({}) on {} {} / {}, engine home {:?}, log dir {:?}",
                env!("CARGO_PKG_VERSION"),
                env!("GIT_SHA"),
                std::env::consts::OS,
                report::os_version(),
                std::env::consts::ARCH,
                workspace::engine_home(app.handle()),
                app.path().app_log_dir(),
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            run_task,
            import_workshop_tools,
            open_logs,
            send_report,
            reveal_report,
            log_ui
        ])
        .run(tauri::generate_context!())
        .expect("error while running D2Turbo");
}
