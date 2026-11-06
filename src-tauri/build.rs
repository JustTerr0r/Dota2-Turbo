use std::process::Command;

fn main() {
    // Short commit hash shown in the app footer as the build id
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    println!(
        "cargo:rustc-env=GIT_SHA={sha}{}",
        if dirty { "-dirty" } else { "" }
    );
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    // Report endpoint baked in by report.rs
    println!("cargo:rerun-if-env-changed=D2TURBO_REPORT_URL");

    tauri_build::build()
}
