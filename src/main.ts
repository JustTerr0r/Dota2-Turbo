import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { logUi } from "./lib/api";

// UI errors go into the app log, so they are part of bug reports
window.addEventListener("error", (e) => logUi("error", `${e.message} at ${e.filename}:${e.lineno}:${e.colno}`));
window.addEventListener("unhandledrejection", (e) => logUi("error", `unhandled rejection: ${String(e.reason)}`));

export default mount(App, { target: document.getElementById("app")! });
