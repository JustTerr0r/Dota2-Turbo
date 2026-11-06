<script lang="ts">
  import { onMount, tick } from "svelte";
  import { ask, message, open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    getStatus,
    importWorkshopTools,
    onEngineEvent,
    onTaskDone,
    logUi,
    openLogs,
    revealReport,
    runTask,
    sendReport,
    type AppStatus,
    type EngineEvent,
    type ReportResult,
    type Task,
  } from "./lib/api";
  import { guidesFor } from "./lib/guides";

  type LogLine = { level: string; text: string };
  type Item = { id: string; label: string; ok: boolean; detail: string; blocking: boolean };

  // Engine localization keys that mark the patch phases
  const STEPS = [
    { id: "install", label: "Подготовка модов", keys: ["installing_terminal"] },
    { id: "decompile", label: "Распаковка файлов игры", keys: ["decompiling_terminal"], workshop: true },
    { id: "rescomp", label: "Компиляция интерфейса", keys: ["compiling_resource_terminal"], workshop: true },
    { id: "pack", label: "Сборка VPK", keys: ["compiling_terminal"] },
    { id: "done", label: "Готово", keys: ["success_terminal"] },
  ];

  let status = $state<AppStatus | null>(null);
  let statusError = $state("");
  let loading = $state(true);
  let running = $state<Task | null>(null);
  let result = $state<{ task: Task; success: boolean } | null>(null);
  let seenKeys = $state<string[]>([]);
  let current = $state("");
  let log = $state<LogLine[]>([]);
  let expanded = $state<string | null>(null);
  let importing = $state(false);
  let logEl: HTMLElement | undefined = $state();

  const CONTACT_KEY = "d2turbo.report.contact";
  let reportOpen = $state(false);
  let reportComment = $state("");
  let reportContact = $state(localStorage.getItem(CONTACT_KEY) ?? "");
  let reporting = $state(false);
  let report = $state<ReportResult | null>(null);
  let reportError = $state("");

  const guides = $derived(guidesFor(status?.platform ?? ""));

  const items = $derived.by<Item[]>(() => {
    if (!status) return [];
    const e = status.engine;
    const dep = (id: string) => status!.deps.find((d) => d.id === id);
    const list: Item[] = [
      {
        id: "steam",
        label: "Steam и Dota 2",
        ok: e.dota_found,
        detail: e.dota_found ? e.steam_library : "Dota 2 не найдена",
        blocking: true,
      },
      { id: "closed", label: "Dota 2 закрыта", ok: !e.dota_running, detail: "", blocking: true },
    ];
    for (const id of ["rosetta", "wine", "dxvk"]) {
      const d = dep(id);
      if (d && guides[id]) list.push({ id, label: guides[id].title, ok: d.ok, detail: d.ok ? d.detail : "", blocking: false });
    }
    list.push({ id: "workshop", label: guides.workshop.title, ok: e.workshop_installed, detail: "", blocking: false });
    return list;
  });

  const blocked = $derived(items.some((i) => i.blocking && !i.ok));
  const degraded = $derived(items.some((i) => !i.blocking && !i.ok));
  const workshopOn = $derived(!!status?.engine.workshop_installed);
  const steps = $derived(STEPS.filter((s) => !s.workshop || workshopOn));

  function stepState(i: number): "done" | "active" | "todo" {
    const reached = (s: (typeof STEPS)[number]) => s.keys.some((k) => seenKeys.includes(k));
    const last = steps.findLastIndex(reached);
    if (i < last || (i === last && steps[i].id === "done")) return "done";
    if (i === last) return running ? "active" : "done";
    return "todo";
  }

  async function refresh() {
    loading = true;
    statusError = "";
    try {
      status = await getStatus();
    } catch (e) {
      statusError = String(e);
      logUi("error", `get_status: ${statusError}`);
    } finally {
      loading = false;
    }
  }

  async function start(task: Task) {
    if (task === "uninstall") {
      const yes = await ask("Удалить установленные моды D2Turbo из Dota 2?", {
        title: "Uninstall",
        kind: "warning",
        okLabel: "Удалить",
        cancelLabel: "Отмена",
      });
      if (!yes) return;
    }
    log = [];
    seenKeys = [];
    current = "";
    result = null;
    running = task;
    try {
      await runTask(task);
    } catch (e) {
      running = null;
      result = { task, success: false };
      log.push({ level: "error", text: String(e) });
      logUi("error", `run_task ${task}: ${String(e)}`);
    }
  }

  function onEvent(ev: EngineEvent) {
    if (ev.type === "log") {
      if (ev.key) seenKeys.push(ev.key);
      if (ev.text) {
        log.push({ level: ev.level, text: ev.text });
        if (ev.level === "info") current = ev.text.replace(/^->\s*/, "");
      }
    } else if (ev.type === "download" && ev.total_bytes > 0) {
      current = `${ev.name}: ${Math.round((ev.downloaded_bytes / ev.total_bytes) * 100)}%`;
    }
    tick().then(() => logEl?.scrollTo({ top: logEl.scrollHeight }));
  }

  async function pickWorkshop() {
    const dir = await open({ directory: true, title: "Папка rescomproot" });
    if (typeof dir !== "string") return;
    importing = true;
    try {
      await importWorkshopTools(dir);
      await refresh();
    } catch (e) {
      await message(String(e), { title: "Импорт не удался", kind: "error" });
    } finally {
      importing = false;
    }
  }

  function openReport() {
    reportOpen = true;
    report = null;
    reportError = "";
    tick().then(() => document.getElementById("report")?.scrollIntoView({ behavior: "smooth" }));
  }

  async function submitReport() {
    reporting = true;
    report = null;
    reportError = "";
    localStorage.setItem(CONTACT_KEY, reportContact.trim());
    try {
      report = await sendReport({ comment: reportComment, contact: reportContact, last_result: result });
      if (report.sent) reportComment = "";
    } catch (e) {
      reportError = String(e);
    } finally {
      reporting = false;
    }
  }

  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
  }

  onMount(() => {
    refresh();
    const unlisten = [
      onEngineEvent(onEvent),
      onTaskDone((d) => {
        running = null;
        result = { task: d.task, success: d.success };
        refresh();
      }),
    ];
    return () => unlisten.forEach((p) => p.then((f) => f()));
  });
</script>

<main>
  <header>
    <h1>D2Turbo</h1>
    <span class="badge">beta</span>
  </header>

  <section class="card">
    <div class="card-head">
      <h2>Готовность</h2>
      <button class="link" onclick={refresh} disabled={loading || !!running}>
        {loading ? "Проверка…" : "Проверить снова"}
      </button>
    </div>

    {#if statusError}
      <p class="error">{statusError}</p>
    {/if}

    <ul class="checks">
      {#each items as item (item.id)}
        <li class:ok={item.ok} class:bad={!item.ok && item.blocking} class:warn={!item.ok && !item.blocking}>
          <div class="row">
            <span class="dot">{item.ok ? "✓" : item.blocking ? "✕" : "!"}</span>
            <span class="label">{item.label}</span>
            {#if !item.ok && guides[item.id]}
              <button class="link" onclick={() => (expanded = expanded === item.id ? null : item.id)}>
                {expanded === item.id ? "Скрыть" : "Как установить"}
              </button>
            {/if}
          </div>
          {#if item.detail}
            <div class="detail">{item.detail}</div>
          {/if}
          {#if expanded === item.id && guides[item.id]}
            {@const g = guides[item.id]}
            <div class="guide">
              <p class="why">{g.why}</p>
              <ol>
                {#each g.steps as step}<li>{step}</li>{/each}
              </ol>
              {#if g.command}
                <pre>{g.command}</pre>
              {/if}
              <div class="actions">
                {#if g.command}
                  <button class="small" onclick={() => copy(g.command!)}>Скопировать команду</button>
                {/if}
                {#if g.link}
                  <button class="small" onclick={() => openUrl(g.link!.url)}>{g.link.label}</button>
                {/if}
                {#if item.id === "workshop"}
                  <button class="small" onclick={pickWorkshop} disabled={importing}>
                    {importing ? "Копирование…" : "Импортировать папку"}
                  </button>
                {/if}
              </div>
            </div>
          {/if}
        </li>
      {/each}
    </ul>

    {#if degraded && !blocked}
      <p class="note">Без отмеченных компонентов установятся гайды и название сборки, но тиры героев и рейтинг лиги — нет.</p>
    {/if}
  </section>

  <section class="primary">
    <button class="patch" onclick={() => start("patch")} disabled={loading || blocked || !!running}>
      {running === "patch" ? "Патчим…" : "Patch"}
    </button>
    <button class="secondary" onclick={() => start("uninstall")} disabled={loading || !status?.engine.dota_found || !!running}>
      {running === "uninstall" ? "Удаляем…" : "Uninstall"}
    </button>
    <p class="hint">При необходимости D2Turbo поправит параметры запуска Dota 2 (-language) и перезапустит Steam.</p>
  </section>

  {#if running || result || log.length}
    <section class="card">
      {#if running === "patch" || result?.task === "patch"}
        <ol class="steps">
          {#each steps as step, i (step.id)}
            <li class={stepState(i)}>{step.label}</li>
          {/each}
        </ol>
      {/if}
      {#if running}
        <p class="current">{current || "Запуск…"}</p>
      {:else if result}
        <p class={result.success ? "success" : "error"}>
          {#if result.task === "patch"}
            {result.success ? "Готово! Запускайте Dota 2." : "Patch не удался."}
          {:else}
            {result.success ? "Моды удалены." : "Uninstall не удался."}
          {/if}
        </p>
      {/if}
      <details>
        <summary>Лог</summary>
        <div class="log" bind:this={logEl}>
          {#each log as line}
            <div class={line.level}>{line.text}</div>
          {/each}
        </div>
      </details>
      <div class="actions">
        {#if result && !result.success && !running}
          <button class="small" onclick={openReport}>Отправить лог разработчику</button>
        {/if}
        <button class="link" onclick={openLogs}>Открыть папку логов</button>
      </div>
    </section>
  {/if}

  {#if reportOpen}
    <section class="card report" id="report">
      <div class="card-head">
        <h2>Поделиться логом</h2>
        <button class="link" onclick={() => (reportOpen = false)} disabled={reporting}>Закрыть</button>
      </div>
      {#if report?.sent}
        <p class="success">Отправлено, спасибо!</p>
        <p class="hint left">
          Код отчёта: <b class="code">{report.id}</b>
          <button class="link" onclick={() => copy(report!.id)}>Скопировать</button>
        </p>
        <p class="hint left">Назовите этот код разработчику, если пишете ему напрямую.</p>
      {:else}
        <label>
          Что случилось?
          <textarea bind:value={reportComment} rows="3" maxlength="2000" placeholder="Например: Patch падает на «Компиляция интерфейса»" disabled={reporting}></textarea>
        </label>
        <label>
          Как с вами связаться (необязательно)
          <input bind:value={reportContact} maxlength="200" placeholder="Telegram / Discord" disabled={reporting} />
        </label>
        <p class="hint left">
          В отчёт попадут логи приложения и движка, настройки D2Turbo, версия системы и строки [D2Turbo] из console.log Dota 2.
          Имя пользователя в путях заменяется на ~.
        </p>
        {#if report && !report.sent}
          <p class="error">Не удалось отправить: {report.error}</p>
          <p class="hint left">
            Отчёт {report.id} сохранён на диске, его можно отправить разработчику вручную.
            <button class="link" onclick={() => revealReport(report!.path)}>Показать файл</button>
          </p>
        {/if}
        {#if reportError}
          <p class="error">{reportError}</p>
        {/if}
        <div class="actions">
          <button class="small primary-small" onclick={submitReport} disabled={reporting}>
            {reporting ? "Отправка…" : report && !report.sent ? "Попробовать снова" : "Отправить"}
          </button>
        </div>
      {/if}
    </section>
  {/if}

  <footer>
    {#if status}
      D2Turbo {status.app_version} ({status.build}) · engine {status.engine.version}
    {/if}
    <div><button class="link" onclick={openReport} disabled={reportOpen}>Поделиться логом с разработчиком</button></div>
  </footer>
</main>
