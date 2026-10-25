<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    api,
    errorMessage,
    IMPORT_FINISHED_EVENT,
    IMPORT_PROGRESS_EVENT,
    type ImportFinished,
    type ImportProgress,
    type ImportReport,
    type Rule,
  } from "$lib/api";
  import Icon from "$lib/Icon.svelte";
  import { normalizeFolder } from "$lib/markdown";
  import RulesEditor from "$lib/RulesEditor.svelte";

  type Props = {
    /** Called after a real import wrote notes. */
    onimported: () => void;
  };
  let { onimported }: Props = $props();

  type Tab = "samples" | "warnings" | "folders" | "links" | "skipped" | "unsupported";

  let page = $state<"import" | "rules">("import");
  let rules = $state<Rule[]>([]);
  let source = $state("");
  // The vault folder to import into; "" is the vault root.
  let target = $state("");
  // Once edited by hand, the target no longer follows the source folder's name.
  let targetEdited = $state(false);
  // Notes already under the target, so the import would merge into them.
  let existingInTarget = $state(0);
  // An import is only offered for the source and target that were last previewed in full.
  let previewed = $state<string | null>(null);
  let running = $state<"preview" | "import" | null>(null);
  let cancelling = $state(false);
  let progress = $state<ImportProgress | null>(null);
  let report = $state<ImportReport | null>(null);
  let tab = $state<Tab>("samples");
  let sampleIndex = $state(0);
  let error = $state("");

  const sample = $derived(report?.samples[sampleIndex] ?? null);
  const normalizedTarget = $derived(normalizeFolder(target));
  const previewKey = $derived(`${source}\n${normalizedTarget}`);
  const canImport = $derived(!running && source.trim() !== "" && previewed === previewKey);

  const tabs = $derived.by(() => {
    if (!report) return [];
    const all: { id: Tab; label: string; count: number | null; warn?: boolean }[] = [
      { id: "samples", label: "Samples", count: null },
      { id: "warnings", label: "Warnings", count: report.warningCount, warn: true },
      { id: "folders", label: "Folders", count: report.folderCount },
      { id: "links", label: "Ambiguous links", count: report.ambiguousLinks.length },
      { id: "skipped", label: "Skipped", count: report.skippedCount },
      { id: "unsupported", label: "Not supported", count: unsupportedCount(report) },
    ];
    return all.filter((t) => t.count === null || t.count > 0);
  });

  const stats = $derived.by(() => {
    if (!report) return [];
    const unsupported = unsupportedCount(report);
    return [
      { label: report.dryRun ? "Will be imported" : "Imported", value: report.imported, sub: "notes", tone: "accent" },
      { label: "Already in vault", value: report.alreadyExist, sub: "same folder and title", tone: "" },
      {
        label: "Not supported yet",
        value: unsupported,
        sub: report.unsupported.map((u) => `${u.format} ${u.count.toLocaleString()}`).join(" · ") || "files",
        tone: "",
      },
      { label: "Skipped", value: report.skippedCount, sub: "files", tone: "" },
      { label: "Folders", value: report.folderCount, sub: "in the vault", tone: "" },
      { label: "Warnings", value: report.warningCount, sub: "imported with notes", tone: "warn" },
    ];
  });

  // Counted whenever the target changes; a stale answer is dropped.
  $effect(() => {
    const folder = normalizedTarget;
    if (!folder) {
      existingInTarget = 0;
      return;
    }
    let current = true;
    api.importTargetNotes(folder).then(
      (n) => current && (existingInTarget = n),
      () => current && (existingInTarget = 0),
    );
    return () => (current = false);
  });

  onMount(() => {
    void loadRules();
    const stops = [
      listen<ImportProgress>(IMPORT_PROGRESS_EVENT, (e) => (progress = e.payload)),
      listen<ImportFinished>(IMPORT_FINISHED_EVENT, (e) => finished(e.payload)),
    ];
    return () => stops.forEach((s) => void s.then((stop) => stop()));
  });

  async function loadRules() {
    rules = await api.importRules().catch(() => rules);
  }

  function showImport() {
    page = "import";
    // The rules may have been edited.
    void loadRules();
  }

  function ruleTarget(rule: Rule): string {
    if (rule.target.startsWith("attribute:")) return `attribute ${rule.target.slice("attribute:".length)}`;
    return { tag: "tag", author: "author", author_email: "author email", topic: "topic", created_at: "created" }[
      rule.target
    ] ?? rule.target;
  }

  function unsupportedCount(r: ImportReport): number {
    return r.unsupported.reduce((n, u) => n + u.count, 0);
  }

  async function chooseSource() {
    const dir = await open({ directory: true, multiple: false, title: "Folder to import" });
    if (typeof dir !== "string") return;
    source = dir;
    targetEdited = false;
    await sourceChanged();
  }

  async function sourceChanged() {
    invalidate();
    if (targetEdited) return;
    const from = source;
    const suggested = from.trim() ? await api.suggestImportTarget(from) : "";
    if (source === from && !targetEdited) target = suggested;
  }

  function targetChanged() {
    targetEdited = true;
    invalidate();
  }

  function invalidate() {
    previewed = null;
    report = null;
    error = "";
  }

  async function start(dryRun: boolean) {
    if (!source.trim() || running) return;
    error = "";
    progress = null;
    cancelling = false;
    // Set first: a small import can finish before the call returns.
    running = dryRun ? "preview" : "import";
    try {
      await api.startImport(source, normalizedTarget, dryRun);
    } catch (e) {
      running = null;
      error = errorMessage(e);
    }
  }

  async function cancel() {
    cancelling = true;
    await api.cancelImport();
  }

  function finished(result: ImportFinished) {
    const wasImport = running === "import";
    running = null;
    if (result.status === "failed") {
      error = errorMessage(result.error);
      return;
    }
    report = result.report;
    sampleIndex = 0;
    tab = "samples";
    if (result.report.dryRun) {
      previewed = result.report.cancelled ? null : `${source}\n${result.report.target}`;
    } else {
      previewed = null;
      if (wasImport && result.report.imported > 0) onimported();
    }
  }

  function plural(n: number, one: string, many = `${one}s`) {
    return `${n.toLocaleString()} ${n === 1 ? one : many}`;
  }
</script>

{#snippet messages(items: { file: string; text: string }[])}
  <ul class="messages">
    {#each items as m, i (i)}<li><span class="file">{m.file}</span> {m.text}</li>{/each}
  </ul>
{/snippet}

{#snippet more(total: number, listed: number)}
  {#if total > listed}
    <p class="muted">And {(total - listed).toLocaleString()} more.</p>
  {/if}
{/snippet}

<div class="import">
  {#if page === "rules"}
    <div class="intro">
      <button class="link back" onclick={showImport}>
        <Icon name="arrowLeft" />
        Back to import
      </button>
      <h1>Label and attribute rules</h1>
    </div>
    <RulesEditor />
  {:else}
    <div class="intro">
      <h1>Import notes</h1>
      <p>
        Reads a folder of notes into this vault: Markdown, text, HTML and RTF, in any common encoding. The source folder is
        never changed and nothing is written until you press Import.
      </p>
    </div>

    <div class="setup">
      <section class="card source">
        <h2>Source and target</h2>
        <div class="fields">
          <div class="field wide">
            <label for="import-source">Source folder</label>
            <div class="row">
              <input
                id="import-source"
                class="mono"
                bind:value={source}
                oninput={sourceChanged}
                placeholder="Folder to import"
                spellcheck="false"
                disabled={!!running}
              />
              <button onclick={chooseSource} disabled={!!running}>Choose…</button>
            </div>
          </div>
          <div class="field">
            <label for="import-target">Target folder in vault</label>
            <input
              id="import-target"
              class="mono"
              bind:value={target}
              oninput={targetChanged}
              placeholder="Vault root"
              spellcheck="false"
              disabled={!!running}
            />
            <span class="hint">Leave empty to import into the vault root.</span>
          </div>
        </div>
        {#if existingInTarget > 0}
          <p class="callout" role="status">
            <Icon name="warning" strokeWidth={2} />
            <span>
              <strong>{normalizedTarget} already holds {plural(existingInTarget, "note")}.</strong> This import merges into
              it. Notes whose folder and title already exist are skipped.
            </span>
          </p>
        {/if}
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        {#if !report && !running}
          <div class="start">
            <button class="primary" onclick={() => start(true)} disabled={!source.trim()}>Preview import</button>
          </div>
        {/if}
      </section>

      <section class="card rules">
        <div class="rules-head">
          <h2>Label and attribute rules</h2>
          <button class="small" onclick={() => (page = "rules")}>Edit rules</button>
        </div>
        {#each rules as rule, i (i)}
          <div class="rule" class:off={!rule.enabled}>
            <span class="rule-name">{rule.name}</span>
            <Icon name="arrowRight" size={14} />
            <span class="rule-target">{ruleTarget(rule)}</span>
            <span class="rule-note">{rule.enabled ? (rule.strip ? "stripped" : "") : "off"}</span>
          </div>
        {:else}
          <p class="muted">No rules. Notes are imported as they are.</p>
        {/each}
      </section>
    </div>

    {#if running}
      <section class="card progress" aria-live="polite">
        <span>{running === "preview" ? "Previewing" : "Importing"}…</span>
        <progress max={progress?.total || 1} value={progress?.done ?? 0}></progress>
        <span class="muted">{progress ? `${progress.done.toLocaleString()} / ${progress.total.toLocaleString()}` : ""}</span>
        <button onclick={cancel} disabled={cancelling}>{cancelling ? "Cancelling…" : "Cancel"}</button>
      </section>
    {/if}

    {#if report && !running}
      <section class="card report">
        <div class="report-head">
          <h2>{report.dryRun ? "Dry run" : "Import finished"}</h2>
          {#if report.cancelled}
            <span class="badge cancelled">Cancelled</span>
          {:else}
            <span class="badge"><Icon name="check" size={12} strokeWidth={3} />Complete</span>
          {/if}
          <span class="muted">
            {plural(report.files, "file")} read · into {report.target ? report.target : "the vault root"}
          </span>
        </div>

        <div class="stats">
          {#each stats as s (s.label)}
            <div class="stat">
              <span class="stat-label">{s.label}</span>
              <span class="stat-value {s.tone}">{s.value.toLocaleString()}</span>
              <span class="stat-sub">{s.sub}</span>
            </div>
          {/each}
        </div>

        <div class="tabs" role="tablist" aria-label="Import details">
          {#each tabs as t (t.id)}
            <button role="tab" aria-selected={tab === t.id} onclick={() => (tab = t.id)}>
              {t.label}
              {#if t.count !== null}<span class:warn={t.warn}>{t.count.toLocaleString()}</span>{/if}
            </button>
          {/each}
        </div>

        <div class="details">
          {#if tab === "samples"}
            {#if sample}
              <div class="sample-head">
                <span class="mono">{sample.path}</span>
                <Icon name="arrowRight" size={14} />
                <span class="mono accent">{sample.folder || "(vault root)"}</span>
                <span class="spacer"></span>
                <span class="muted">{sampleIndex + 1} of {report.samples.length} changed files</span>
                <button
                  class="small icon"
                  aria-label="Previous sample"
                  disabled={sampleIndex === 0}
                  onclick={() => sampleIndex--}
                >
                  <Icon name="chevronLeft" size={14} strokeWidth={2} />
                </button>
                <button
                  class="small icon"
                  aria-label="Next sample"
                  disabled={sampleIndex >= report.samples.length - 1}
                  onclick={() => sampleIndex++}
                >
                  <Icon name="chevronRight" size={14} strokeWidth={2} />
                </button>
              </div>
              <div class="compare">
                <figure>
                  <figcaption>Before · source file</figcaption>
                  <pre class="before">{sample.before}</pre>
                </figure>
                <figure class="after">
                  <figcaption>After · note in vault</figcaption>
                  <pre>{sample.after}</pre>
                </figure>
              </div>
            {:else}
              <p class="muted">No files needed changes.</p>
            {/if}
          {:else if tab === "warnings"}
            {@render messages(report.warnings.map((m) => ({ file: m.path, text: m.message })))}
            {@render more(report.warningCount, report.warnings.length)}
          {:else if tab === "folders"}
            {@render messages(report.folders.map((f) => ({ file: f.folder || "(vault root)", text: plural(f.count, "note") })))}
            {@render more(report.folderCount, report.folders.length)}
          {:else if tab === "links"}
            <p class="muted">These links match more than one note. Open the note and pick the one meant.</p>
            {@render messages(report.ambiguousLinks.map((l) => ({ file: l.note, text: `[[${l.target}]]` })))}
          {:else if tab === "skipped"}
            {@render messages(report.skipped.map((m) => ({ file: m.path, text: m.message })))}
            {@render more(report.skippedCount, report.skipped.length)}
          {:else}
            <p class="muted">Hush can't read these formats yet (Word documents, PDFs, images, …). The files stay in the folder.</p>
            <ul class="chips">
              {#each report.unsupported as u (u.format)}
                <li class="chip outline">{u.format} {u.count.toLocaleString()}</li>
              {/each}
            </ul>
          {/if}
        </div>

        <div class="report-foot">
          <span class="foot-text">
            {#if report.dryRun}
              This preview matches the current source and target. Changing either needs a new preview.
            {:else}
              {plural(report.imported, "note")} imported.
            {/if}
          </span>
          <button onclick={() => start(true)} disabled={!source.trim()}>Run preview again</button>
          {#if report.dryRun}
            <button class="primary" onclick={() => start(false)} disabled={!canImport}>
              <Icon name="import" />
              Import {plural(report.imported, "note")}
            </button>
          {/if}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .import {
    max-width: 1080px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  .intro {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
  }

  h1 {
    margin: 0;
    font-size: 26px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  p {
    margin: 0;
  }

  .intro p {
    color: var(--text-2);
  }

  .back {
    gap: 6px;
    margin-bottom: 8px;
  }

  .setup {
    display: flex;
    flex-wrap: wrap;
    gap: 20px;
  }

  .card {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 20px;
  }

  .source {
    flex: 3 1 520px;
  }

  .rules {
    flex: 2 1 340px;
    gap: 0;
  }

  .fields {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
  }

  .field {
    flex: 1 1 220px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field.wide {
    flex-basis: 260px;
  }

  .field > label {
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
  }

  .field input {
    min-height: 36px;
    padding: 0 10px;
    border-radius: 8px;
    font-size: 13px;
  }

  .row {
    display: flex;
    gap: 8px;
  }

  .row input {
    flex: 1;
    min-width: 0;
  }

  .row button {
    min-height: 36px;
    background: transparent;
  }

  .hint {
    font-size: 12px;
    color: var(--muted);
  }

  .start {
    display: flex;
    justify-content: flex-end;
  }

  .start button {
    min-height: 36px;
    padding: 0 16px;
  }

  .rules-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: 10px;
  }

  .rule {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    border-top: 1px solid var(--border);
    font-size: 13px;
  }

  .rule :global(svg) {
    color: var(--muted);
  }

  .rule.off {
    opacity: 0.6;
  }

  .rule-name {
    flex: 0 1 140px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-2);
  }

  .rule-target {
    flex: 1;
    min-width: 0;
  }

  .rule-note {
    font-size: 12px;
    color: var(--muted);
  }

  .progress {
    display: grid;
    grid-template-columns: auto 1fr auto auto;
    align-items: center;
  }

  progress {
    width: 100%;
    accent-color: var(--accent);
  }

  .report {
    gap: 0;
    padding: 0;
    overflow: hidden;
  }

  .report-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 8px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-soft-text);
    font-size: 12px;
    font-weight: 500;
  }

  .badge.cancelled {
    background: var(--warn-soft);
    color: var(--warn);
  }

  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(160px, 100%), 1fr));
    border-bottom: 1px solid var(--border);
  }

  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 14px 20px;
    border-right: 1px solid var(--border);
  }

  .stat-label {
    font-size: 12px;
    color: var(--muted);
  }

  .stat-value {
    font-size: 22px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .stat-value.accent {
    color: var(--accent);
  }

  .stat-value.warn {
    color: var(--warn);
  }

  .stat-sub {
    font-size: 12px;
    color: var(--text-2);
  }

  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 0 12px;
    border-bottom: 1px solid var(--border);
  }

  .tabs button {
    gap: 6px;
    min-height: 40px;
    padding: 0 10px;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: transparent;
    color: var(--text-2);
    font-size: 13px;
  }

  .tabs button[aria-selected="true"] {
    border-bottom-color: var(--accent);
    color: var(--text);
    font-weight: 500;
  }

  .tabs .warn {
    color: var(--warn);
  }

  .details {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 20px 20px;
  }

  .sample-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }

  .sample-head :global(svg) {
    color: var(--muted);
  }

  .sample-head button :global(svg) {
    color: inherit;
  }

  .sample-head .muted {
    font-size: 12px;
  }

  .accent {
    color: var(--accent);
  }

  .spacer {
    flex: 1;
  }

  .compare {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }

  figure {
    flex: 1 1 380px;
    min-width: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
  }

  figcaption {
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    font-size: 12px;
    color: var(--muted);
  }

  figure.after {
    border-color: var(--accent);
  }

  .after figcaption {
    background: var(--accent-soft);
    color: var(--accent-soft-text);
  }

  pre {
    margin: 0;
    max-height: 50vh;
    overflow: auto;
    padding: 12px;
    background: var(--code);
    font-family: var(--mono);
    font-size: 12.5px;
    line-height: 1.7;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  pre.before {
    color: var(--text-2);
  }

  .messages {
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 40vh;
    overflow-y: auto;
    font-size: 13px;
    border: 1px solid var(--border);
    border-radius: 8px;
  }

  .messages li {
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    overflow-wrap: anywhere;
  }

  .messages li:last-child {
    border-bottom: none;
  }

  .file {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-2);
  }

  .report-foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 14px 20px;
    border-top: 1px solid var(--border);
    background: var(--panel);
  }

  .foot-text {
    flex: 1 1 260px;
    font-size: 13px;
    color: var(--text-2);
  }

  .report-foot button {
    min-height: 36px;
    padding: 0 14px;
  }
</style>
