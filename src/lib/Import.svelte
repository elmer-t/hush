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
  } from "$lib/api";
  import RulesEditor from "$lib/RulesEditor.svelte";

  type Props = {
    /** Called after a real import wrote notes. */
    onimported: () => void;
  };
  let { onimported }: Props = $props();

  let tab = $state<"import" | "rules">("import");
  let source = $state("");
  // An import is only offered for the folder that was last previewed in full.
  let previewed = $state<string | null>(null);
  let running = $state<"preview" | "import" | null>(null);
  let cancelling = $state(false);
  let progress = $state<ImportProgress | null>(null);
  let report = $state<ImportReport | null>(null);
  let sampleIndex = $state(0);
  let error = $state("");

  const sample = $derived(report?.samples[sampleIndex] ?? null);
  const canImport = $derived(!running && source.trim() !== "" && previewed === source);

  onMount(() => {
    const stops = [
      listen<ImportProgress>(IMPORT_PROGRESS_EVENT, (e) => (progress = e.payload)),
      listen<ImportFinished>(IMPORT_FINISHED_EVENT, (e) => finished(e.payload)),
    ];
    return () => stops.forEach((s) => void s.then((stop) => stop()));
  });

  async function chooseSource() {
    const dir = await open({ directory: true, multiple: false, title: "Folder to import" });
    if (typeof dir !== "string") return;
    source = dir;
    sourceChanged();
  }

  function sourceChanged() {
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
      await api.startImport(source, dryRun);
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
    if (result.report.dryRun) {
      previewed = result.report.cancelled ? null : source;
    } else {
      previewed = null;
      if (wasImport && result.report.imported > 0) onimported();
    }
  }

  function plural(n: number, one: string, many = `${one}s`) {
    return `${n.toLocaleString()} ${n === 1 ? one : many}`;
  }
</script>

<div class="import">
  <div class="tabs" role="tablist">
    <button role="tab" aria-selected={tab === "import"} onclick={() => (tab = "import")}>Import</button>
    <button role="tab" aria-selected={tab === "rules"} onclick={() => (tab = "rules")}>Label rules</button>
  </div>

  {#if tab === "rules"}
    <RulesEditor />
  {:else}
    <section class="source">
      <p class="muted">
        Imports every readable file in a folder, such as an Obsidian vault or an old archive: Markdown, text, HTML and
        RTF, in any common encoding. Notes keep their folders. HTML is converted, hard-wrapped lines are joined, data
        files are kept as they are, and the <button class="link" onclick={() => (tab = "rules")}
          >label rules</button
        > fill in tags, author and dates. The folder itself is never changed.
      </p>
      <div class="row">
        <input
          class="path"
          bind:value={source}
          oninput={sourceChanged}
          placeholder="Folder to import"
          aria-label="Folder to import"
          spellcheck="false"
          disabled={!!running}
        />
        <button onclick={chooseSource} disabled={!!running}>Choose folder…</button>
      </div>
      <div class="row">
        <button onclick={() => start(true)} disabled={!source.trim() || !!running}>Preview</button>
        <button
          class="primary"
          onclick={() => start(false)}
          disabled={!canImport}
          title={canImport ? "" : "Preview the folder first"}
        >
          Import{#if canImport && report?.dryRun}&nbsp;{plural(report.imported, "note")}{/if}
        </button>
      </div>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    </section>

    {#if running}
      <section class="progress" aria-live="polite">
        <span>{running === "preview" ? "Previewing" : "Importing"}…</span>
        <progress max={progress?.total || 1} value={progress?.done ?? 0}></progress>
        <span class="muted">{progress ? `${progress.done} / ${progress.total}` : ""}</span>
        <button onclick={cancel} disabled={cancelling}>{cancelling ? "Cancelling…" : "Cancel"}</button>
      </section>
    {/if}

    {#if report && !running}
      <section class="report">
        <h2>{report.dryRun ? "Preview" : "Import finished"}</h2>
        <p>
          {plural(report.files, "file")} found.
          {report.dryRun ? "Would import" : "Imported"}
          {plural(report.imported, "note")}{#if report.skipped.length}, skipping {report.skipped.length}{/if}.
          {#if report.cancelled}<strong>Cancelled</strong> before all files were done.{/if}
        </p>

        {#if report.samples.length > 0}
          <div class="samples">
            <label>
              Sample ({report.samples.length} changed files)
              <select bind:value={sampleIndex}>
                {#each report.samples as s, i (s.path)}<option value={i}>{s.path}</option>{/each}
              </select>
            </label>
            {#if sample}
              <div class="compare">
                <div>
                  <h3>Before</h3>
                  <pre>{sample.before}</pre>
                </div>
                <div>
                  <h3>After</h3>
                  <pre>{sample.after}</pre>
                </div>
              </div>
            {/if}
          </div>
        {/if}

        {#if report.unsupported.length > 0}
          <h3>Not imported yet</h3>
          <p class="muted">Hush can't read these formats yet (Word documents, PDFs, images, …). The files stay in the folder.</p>
          <ul class="formats">
            {#each report.unsupported as u (u.format)}<li>{u.format} <span class="muted">{u.count.toLocaleString()}</span></li>{/each}
          </ul>
        {/if}

        {#if report.skipped.length > 0}
          <h3>Skipped</h3>
          <ul class="messages">
            {#each report.skipped as m, i (i)}<li><span class="file">{m.path}</span> {m.message}</li>{/each}
          </ul>
        {/if}

        {#if report.ambiguousLinks.length > 0}
          <h3>Ambiguous links</h3>
          <p class="muted">These links match more than one note. Open the note and pick the one meant.</p>
          <ul class="messages">
            {#each report.ambiguousLinks as l, i (i)}<li><span class="file">{l.note}</span> [[{l.target}]]</li>{/each}
          </ul>
        {/if}

        {#if report.warningCount > 0}
          <h3>Warnings ({report.warningCount.toLocaleString()})</h3>
          <ul class="messages">
            {#each report.warnings as m, i (i)}<li><span class="file">{m.path}</span> {m.message}</li>{/each}
          </ul>
          {#if report.warningCount > report.warnings.length}
            <p class="muted">And {(report.warningCount - report.warnings.length).toLocaleString()} more.</p>
          {/if}
        {/if}
      </section>
    {/if}
  {/if}
</div>

<style>
  .import {
    max-width: 960px;
    margin: 0 auto;
    display: grid;
    gap: 1.25rem;
    align-content: start;
  }

  .tabs {
    display: flex;
    gap: 0.25rem;
    border-bottom: 1px solid var(--border);
  }

  .tabs button {
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: none;
  }

  .tabs button[aria-selected="true"] {
    border-bottom-color: var(--accent);
    font-weight: 600;
  }

  section {
    display: grid;
    gap: 0.75rem;
  }

  p {
    margin: 0;
  }

  .link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
    text-decoration: underline;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    min-width: 0;
  }

  .path {
    flex: 1;
    min-width: 0;
    font-family: var(--mono);
    font-size: 0.85em;
  }

  .progress {
    grid-template-columns: auto 1fr auto auto;
    align-items: center;
  }

  progress {
    width: 100%;
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  h3 {
    margin: 0.5rem 0 0;
    font-size: 0.9rem;
  }

  .samples {
    display: grid;
    gap: 0.5rem;
  }

  .compare {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.75rem;
  }

  .compare > div {
    min-width: 0;
  }

  pre {
    margin: 0.25rem 0 0;
    max-height: 50vh;
    overflow: auto;
    padding: 0.6rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    font-family: var(--mono);
    font-size: 0.8em;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .messages {
    margin: 0;
    padding: 0;
    list-style: none;
    max-height: 30vh;
    overflow-y: auto;
    font-size: 0.85em;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .messages li {
    padding: 0.3rem 0.6rem;
    border-bottom: 1px solid var(--border);
    overflow-wrap: anywhere;
  }

  .messages li:last-child {
    border-bottom: none;
  }

  .formats {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .formats li {
    padding: 0.1em 0.6em;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: 0.85em;
  }

  .file {
    font-family: var(--mono);
    color: var(--muted);
  }

  @media (max-width: 700px) {
    .compare {
      grid-template-columns: 1fr;
    }
  }
</style>
