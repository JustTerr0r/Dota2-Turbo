//! "Share the log with the developer": bundles the app log, the engine logs/config and a
//! system summary into a zip, keeps a local copy and uploads it to the report endpoint.
//!
//! Upload contract (multipart/form-data `POST REPORT_URL`):
//! - `meta`: JSON string, the same object as `meta.json` inside the zip
//! - `logs`: the zip file (`<id>.zip`, `application/zip`)
//!
//! Any 2xx response counts as delivered.

use std::collections::hash_map::RandomState;
use std::fs::{self, File};
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use zip::write::SimpleFileOptions;

/// Report endpoint. Override at build time: `D2TURBO_REPORT_URL=https://... npm run tauri build`.
pub const REPORT_URL: &str = match option_env!("D2TURBO_REPORT_URL") {
    Some(url) => url,
    None => "https://d2-rurbo-production.up.railway.app/api/app/reports",
};

/// Only the tail of each log goes into the report
const MAX_FILE_BYTES: u64 = 3 * 1024 * 1024;
/// Newest app log files (current + rotated) included
const APP_LOG_FILES: usize = 3;
/// Local copies kept in `<app log dir>/reports`
const KEEP_REPORTS: usize = 5;
/// Dota's console.log can be huge, only its tail is scanned for the mod's lines
const CONSOLE_SCAN_BYTES: u64 = 16 * 1024 * 1024;
const CONSOLE_TAG: &str = "[D2Turbo]";
const CONSOLE_LINES: usize = 2000;
const ENGINE_CONFIG_FILES: &[&str] = &["minify_config.json", "mods.json"];

#[derive(Debug, Serialize)]
pub struct ReportResult {
    /// Short code the user can quote to the developer
    pub id: String,
    pub sent: bool,
    pub error: Option<String>,
    /// Local copy of the zip, for sending it by hand when the upload failed
    pub path: String,
}

/// Where the report reads its files from.
pub struct Sources {
    pub app_logs: PathBuf,
    pub engine_home: PathBuf,
    pub console_log: Option<PathBuf>,
    /// Replaced by `~` in every text file so the user name doesn't leak
    pub user_home: Option<String>,
}

pub fn new_id() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut h = RandomState::new().build_hasher();
    h.write_u128(now_nanos());
    let n = h.finish();
    let code: String = (0..8)
        .map(|i| ALPHABET[((n >> (i * 5)) & 31) as usize] as char)
        .collect();
    format!("{}-{}", &code[..4], &code[4..])
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}

pub fn unix_now() -> u64 {
    (now_nanos() / 1_000_000_000) as u64
}

pub fn os_version() -> String {
    #[cfg(target_os = "macos")]
    let out = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output();
    #[cfg(windows)]
    let out = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/c", "ver"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let out: std::io::Result<std::process::Output> = Err(std::io::Error::other("unsupported"));
    out.ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Masks the user's home directory (plain and JSON-escaped Windows form).
pub fn sanitize(text: &str, user_home: Option<&str>) -> String {
    let Some(home) = user_home.filter(|h| h.len() > 3) else {
        return text.to_string();
    };
    let mut out = text.replace(home, "~");
    if home.contains('\\') {
        out = out.replace(&home.replace('\\', "\\\\"), "~");
    }
    out
}

/// Last `max` bytes of a file, starting at a line boundary when cut.
fn read_tail(path: &Path, max: u64) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let skipped = len.saturating_sub(max);
    file.seek(SeekFrom::Start(skipped))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    if skipped == 0 {
        return Ok(text.into_owned());
    }
    let body = text.split_once('\n').map_or("", |(_, rest)| rest);
    Ok(format!("[… first {skipped} bytes cut …]\n{body}"))
}

/// Files of a directory, newest first.
fn files_by_mtime(dir: &Path, ext: Option<&str>) -> Vec<PathBuf> {
    let mut files: Vec<(SystemTime, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| ext.is_none_or(|x| p.extension().is_some_and(|e| e == x)))
        .map(|p| {
            let t = p
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            (t, p)
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().map(|(_, p)| p).collect()
}

fn console_lines(path: &Path) -> Option<String> {
    let text = read_tail(path, CONSOLE_SCAN_BYTES).ok()?;
    let lines: Vec<&str> = text.lines().filter(|l| l.contains(CONSOLE_TAG)).collect();
    let start = lines.len().saturating_sub(CONSOLE_LINES);
    Some(lines[start..].join("\n"))
}

/// Writes the report zip to `out`; files that are missing or unreadable are skipped.
pub fn build_zip(src: &Sources, meta: &Value, out: &Path) -> Result<(), String> {
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = File::create(out).map_err(|e| format!("create {}: {e}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let home = src.user_home.as_deref();
    let mut entries: Vec<(String, String)> = vec![(
        "meta.json".into(),
        serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?,
    )];

    let mut add_file = |name: String, path: &Path| match read_tail(path, MAX_FILE_BYTES) {
        Ok(text) => entries.push((name, text)),
        Err(e) => log::warn!("report: skip {}: {e}", path.display()),
    };
    for path in files_by_mtime(&src.app_logs, Some("log"))
        .iter()
        .take(APP_LOG_FILES)
    {
        add_file(format!("app/{}", file_name(path)), path);
    }
    for path in files_by_mtime(&src.engine_home.join("logs"), None) {
        add_file(format!("engine/logs/{}", file_name(&path)), &path);
    }
    for name in ENGINE_CONFIG_FILES {
        let path = src.engine_home.join("config").join(name);
        if path.is_file() {
            add_file(format!("engine/config/{name}"), &path);
        }
    }
    if let Some(text) = src.console_log.as_deref().and_then(console_lines) {
        entries.push(("dota/console_d2turbo.log".into(), text));
    }

    for (name, text) in entries {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(sanitize(&text, home).as_bytes())
            .map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Deletes all but the newest `KEEP_REPORTS` zips.
pub fn prune_reports(dir: &Path) {
    for old in files_by_mtime(dir, Some("zip")).iter().skip(KEEP_REPORTS) {
        let _ = fs::remove_file(old);
    }
}

pub async fn upload(id: &str, meta: &Value, zip_path: &Path) -> Result<(), String> {
    upload_to(REPORT_URL, id, meta, zip_path).await
}

async fn upload_to(url: &str, id: &str, meta: &Value, zip_path: &Path) -> Result<(), String> {
    use reqwest::multipart::{Form, Part};

    let bytes = fs::read(zip_path).map_err(|e| e.to_string())?;
    let part = Part::bytes(bytes)
        .file_name(format!("{id}.zip"))
        .mime_str("application/zip")
        .map_err(|e| e.to_string())?;
    let form = Form::new()
        .text("meta", meta.to_string())
        .part("logs", part);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(format!(
            "D2Turbo/{} ({})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        ))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let body: String = resp
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect();
    Err(format!("HTTP {status}: {body}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn id_is_short_and_random() {
        let (a, b) = (new_id(), new_id());
        assert_eq!(a.len(), 9);
        assert_eq!(&a[4..5], "-");
        assert_ne!(a, b);
    }

    #[test]
    fn masks_user_home() {
        assert_eq!(
            sanitize("/Users/bob/Library/x", Some("/Users/bob")),
            "~/Library/x"
        );
        assert_eq!(
            sanitize(r#"{"p": "C:\\Users\\bob\\x"}"#, Some(r"C:\Users\bob")),
            r#"{"p": "~\\x"}"#
        );
        assert_eq!(sanitize("/x", None), "/x");
    }

    #[test]
    fn tail_starts_at_line_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.log");
        fs::write(&p, "first line\nsecond\nthird\n").unwrap();
        assert_eq!(read_tail(&p, 1000).unwrap(), "first line\nsecond\nthird\n");
        let cut = read_tail(&p, 10).unwrap();
        assert!(cut.ends_with("\nthird\n"), "{cut}");
        assert!(!cut.contains("second"));
    }

    #[test]
    fn builds_zip_with_logs_config_and_console_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let app_logs = root.join("applogs");
        let engine = root.join("engine");
        fs::create_dir_all(&app_logs).unwrap();
        fs::create_dir_all(engine.join("logs")).unwrap();
        fs::create_dir_all(engine.join("config/rescomproot")).unwrap();
        fs::write(app_logs.join("D2Turbo.log"), "start /Users/bob/x\n").unwrap();
        fs::write(app_logs.join("ignored.txt"), "x").unwrap();
        fs::write(engine.join("logs/crashlog.txt"), "boom").unwrap();
        fs::write(engine.join("config/mods.json"), "{}").unwrap();
        let console = root.join("console.log");
        fs::write(&console, "noise\n[D2Turbo] tiers ok\nnoise\n").unwrap();

        let out = root.join("reports/r.zip");
        let src = Sources {
            app_logs,
            engine_home: engine,
            console_log: Some(console),
            user_home: Some("/Users/bob".into()),
        };
        build_zip(&src, &json!({ "id": "TEST" }), &out).unwrap();

        let mut zip = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        let mut names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "app/D2Turbo.log",
                "dota/console_d2turbo.log",
                "engine/config/mods.json",
                "engine/logs/crashlog.txt",
                "meta.json",
            ]
        );
        let mut text = String::new();
        zip.by_name("app/D2Turbo.log")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "start ~/x\n");
        text.clear();
        zip.by_name("dota/console_d2turbo.log")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "[D2Turbo] tiers ok");
    }

    /// One-shot HTTP server: returns the raw request and answers with `status`.
    fn serve_once(status: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/reports", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let (mut head, mut len) = (String::new(), 0usize);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let reply =
                format!("HTTP/1.1 {status}\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
            reader.get_mut().write_all(reply.as_bytes()).unwrap();
            head + &String::from_utf8_lossy(&body)
        });
        (url, handle)
    }

    #[test]
    fn uploads_meta_and_zip_as_multipart() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("r.zip");
        fs::write(&zip_path, "ZIPDATA").unwrap();
        let (url, server) = serve_once("200 OK");
        let meta = json!({ "id": "AAAA-BBBB" });
        tauri::async_runtime::block_on(upload_to(&url, "AAAA-BBBB", &meta, &zip_path)).unwrap();
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /reports"), "{req}");
        assert!(req.contains("multipart/form-data; boundary="));
        assert!(req.contains(r#"name="meta""#) && req.contains(r#"{"id":"AAAA-BBBB"}"#));
        assert!(req.contains(r#"name="logs"; filename="AAAA-BBBB.zip""#));
        assert!(req.contains("application/zip") && req.contains("ZIPDATA"));
    }

    #[test]
    fn non_2xx_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("r.zip");
        fs::write(&zip_path, "x").unwrap();
        let (url, server) = serve_once("500 Internal Server Error");
        let err = tauri::async_runtime::block_on(upload_to(&url, "X", &json!({}), &zip_path))
            .unwrap_err();
        server.join().unwrap();
        assert!(
            err.starts_with("HTTP 500") && err.ends_with("nope"),
            "{err}"
        );
    }

    #[test]
    fn keeps_only_newest_reports() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..KEEP_REPORTS + 2 {
            fs::write(tmp.path().join(format!("{i}.zip")), "x").unwrap();
        }
        prune_reports(tmp.path());
        assert_eq!(files_by_mtime(tmp.path(), Some("zip")).len(), KEEP_REPORTS);
    }
}
