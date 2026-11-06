import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Task = "patch" | "uninstall";

export interface EngineStatus {
  version: string;
  steam_root: string;
  steam_library: string;
  dota_found: boolean;
  dota_running: boolean;
  workshop_installed: boolean;
  output_path: string;
  steam_language: string;
}

export interface DepCheck {
  id: string;
  ok: boolean;
  detail: string;
}

export interface Features {
  turbo_builds: boolean;
  turbo_picks: boolean;
}

export interface AppStatus {
  app_version: string;
  build: string;
  platform: string;
  engine: EngineStatus;
  deps: DepCheck[];
  engine_home: string;
  features: Features;
  language: string;
  languages: string[];
}

export type EngineEvent =
  | { type: "log"; key: string | null; level: "info" | "warning" | "error" | "success"; text: string }
  | { type: "separator" }
  | { type: "download"; name: string; downloaded_bytes: number; total_bytes: number; status: string };

export interface TaskDone {
  task: Task;
  success: boolean;
  code: number | null;
}

export interface ReportResult {
  /** Short code the user quotes to the developer, e.g. "K7QM-3XPA" */
  id: string;
  sent: boolean;
  error: string | null;
  /** Local copy of the zip */
  path: string;
}

export interface ReportRequest {
  comment: string;
  contact: string;
  last_result: { task: Task; success: boolean } | null;
}

export const getStatus = () => invoke<AppStatus>("get_status");
export const runTask = (task: Task) => invoke<void>("run_task", { task });
export const setFeatures = (features: Features) => invoke<void>("set_features", { features });
export const setLanguage = (language: string) => invoke<void>("set_language", { language });
export const importWorkshopTools = (path: string) => invoke<void>("import_workshop_tools", { path });
export const openLogs = () => invoke<void>("open_logs");
export const sendReport = (request: ReportRequest) => invoke<ReportResult>("send_report", { request });
export const revealReport = (path: string) => invoke<void>("reveal_report", { path });

/** Writes into the app log file; never throws, logging must not break the UI. */
export const logUi = (level: "info" | "warn" | "error", message: string) =>
  invoke<void>("log_ui", { level, message }).catch(() => {});

export const onEngineEvent = (cb: (e: EngineEvent) => void): Promise<UnlistenFn> =>
  listen<EngineEvent>("engine://event", (e) => cb(e.payload));
export const onTaskDone = (cb: (d: TaskDone) => void): Promise<UnlistenFn> =>
  listen<TaskDone>("engine://done", (e) => cb(e.payload));
