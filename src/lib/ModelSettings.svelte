<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    api,
    errorMessage,
    MODEL_DOWNLOAD_EVENT,
    type DownloadStatus,
    type ModelInfo,
    type ModelOverview,
  } from "$lib/api";
  import { MODEL_FEATURES, model, reducedText } from "$lib/modelStatus.svelte";

  let overview = $state<ModelOverview | null>(null);
  let error = $state("");
  let busy = $state(false);

  const current = $derived(overview?.models.find((m) => m.id === overview?.model) ?? null);
  // The live status wins over the one loaded with the overview.
  const status = $derived(model.status ?? overview?.status ?? null);
  const reason = $derived(reducedText());
  // Each model's download this run, and the last error per model (import, remove).
  let downloads = $state<Record<string, DownloadStatus>>({});
  let modelErrors = $state<Record<string, string>>({});
  let notices = $state<Record<string, string>>({});

  onMount(() => {
    const unlisten = listen<DownloadStatus>(MODEL_DOWNLOAD_EVENT, (e) => {
      downloads[e.payload.model] = e.payload;
      if (e.payload.state === "done") void load();
    });
    api.modelDownloads().then((all) => all.forEach((d) => (downloads[d.model] = d)), () => {});
    return () => void unlisten.then((stop) => stop());
  });

  // Loads at once, and again when a model finished loading or the ready model changed:
  // sizes and states may differ.
  $effect(() => {
    void model.status?.readyModel;
    void model.status?.load.state;
    void load();
  });

  async function load() {
    try {
      overview = await api.modelOverview();
      error = "";
    } catch (e) {
      error = errorMessage(e);
    }
  }

  const mb = (bytes: number) => `${(bytes / 1_048_576).toFixed(1)} MB`;

  /** `about 40 min`, `about 3 h`, `about 4.6 days`. */
  function about(secs: number): string {
    if (secs < 90) return "about a minute";
    if (secs < 3600) return `about ${Math.round(secs / 60)} min`;
    if (secs < 48 * 3600) return `about ${(secs / 3600).toFixed(secs < 36_000 ? 1 : 0)} h`;
    return `about ${(secs / 86_400).toFixed(1)} days`;
  }

  /** What the vault's model is doing, in a few words. */
  function stateText(): string {
    if (!overview) return "";
    const s = status;
    const i = model.index;
    const indexing = i.state === "indexing" && i.done < i.total ? `indexing ${i.done.toLocaleString()} of ${i.total.toLocaleString()}` : null;
    if (!s || s.model !== overview.model) return indexing ?? "not loaded yet";
    switch (s.load.state) {
      case "missing":
        return `missing (${s.load.file})`;
      case "checksum":
        return `checksum failed (${s.load.file})`;
      case "unknown":
        return "unknown to this version of Hush";
      case "failed":
        return `failed to load: ${s.load.message}`;
      case "loading":
        return "loading";
    }
    if (indexing) return indexing;
    if (s.readyModel !== s.model) return s.load.state === "ready" ? "loaded, indexing" : "not loaded yet";
    return s.load.state === "ready" ? "ready" : "ready (loads when needed)";
  }

  function availabilityText(m: ModelInfo): string {
    switch (m.availability.state) {
      case "available":
        return `on this computer, ${mb(m.availability.bytes)}`;
      case "missing":
        return m.bundled ? `not on this computer (${m.availability.file} is missing)` : "not downloaded";
      case "checksum":
        return `not usable: ${m.availability.file} failed its checksum`;
      case "unreadable":
        return `not usable: ${m.availability.message}`;
    }
  }

  async function use(m: ModelInfo) {
    if (!overview) return;
    const rate = m.rate ?? overview.defaultRate;
    const measured = m.rate != null ? "measured on this computer" : "an estimate, not yet measured on this computer";
    const notes = overview.notes;
    const serving = status?.readyModel
      ? `Until it is done, related notes and tag suggestions keep using ${overview.models.find((x) => x.id === status?.readyModel)?.name ?? status.readyModel}.`
      : "Related notes and tag suggestions start once it is done.";
    const ok = await ask(
      `Switch this vault to ${m.name}? Every note (${notes.toLocaleString()}) is embedded again with it: ` +
        `${about(notes / rate)} at ${rate.toFixed(1)} notes/s (${measured}). ` +
        `${serving} Tag classifiers are trained again for the new model.`,
      { title: "Switch model", kind: "warning", okLabel: "Switch model", cancelLabel: "Cancel" },
    );
    if (!ok) return;
    busy = true;
    try {
      await api.setEmbeddingModel(m.id);
      await load();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }

  const pct = (done: number, total: number) => (total > 0 ? Math.min(100, (done / total) * 100) : 0);

  async function download(m: ModelInfo) {
    modelErrors[m.id] = notices[m.id] = "";
    try {
      await api.downloadModel(m.id);
    } catch (e) {
      modelErrors[m.id] = errorMessage(e);
    }
  }

  async function importFiles(m: ModelInfo) {
    modelErrors[m.id] = notices[m.id] = "";
    const picked = await open({
      multiple: true,
      title: `Model files for ${m.name} (the .onnx file and tokenizer.json)`,
      filters: [{ name: "Model files", extensions: ["onnx", "json"] }],
    });
    if (!picked || picked.length === 0) return;
    busy = true;
    try {
      const stored = await api.importModelFiles(m.id, picked);
      notices[m.id] = `Imported ${stored.join(" and ")}.`;
    } catch (e) {
      modelErrors[m.id] = errorMessage(e);
    } finally {
      busy = false;
      await load();
    }
  }

  async function remove(m: ModelInfo) {
    modelErrors[m.id] = notices[m.id] = "";
    const ok = await ask(`Delete ${m.name}'s files from this computer? You can download or import it again later.`, {
      title: "Remove model",
      kind: "warning",
      okLabel: "Remove",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    busy = true;
    try {
      const freed = await api.removeModel(m.id);
      delete downloads[m.id];
      notices[m.id] = `Removed, ${mb(freed)} freed.`;
    } catch (e) {
      modelErrors[m.id] = errorMessage(e);
    } finally {
      busy = false;
      await load();
    }
  }

  async function recheck() {
    busy = true;
    try {
      await api.recheckModels();
      await load();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="card" id="model-settings">
  <h2>Model</h2>
  <p class="muted">
    The embedding model works out what each note is about, on this computer. Without a ready model these are off:
    {MODEL_FEATURES}. Search and everything else keep working.
  </p>
  <p class="muted">
    Extra models are optional. Hush goes online only when you click Download, and only to that model's address. To
    stay offline, get the files another way and add them with Load model file.
  </p>
  {#if reason}
    <p class="callout" role="status">Off right now: {reason}.</p>
  {/if}

  {#if overview}
    <dl class="current">
      <div><dt>Model</dt><dd>{current?.name ?? overview.model}</dd></div>
      <div><dt>ID</dt><dd class="mono">{overview.model}</dd></div>
      <div><dt>State</dt><dd>{stateText()}</dd></div>
      {#if current}
        <div>
          <dt>On disk</dt>
          <dd>{current.availability.state === "available" ? mb(current.availability.bytes) : "–"}</dd>
        </div>
        <div><dt>Vectors</dt><dd>{current.dims} dimensions, up to {current.maxTokens} tokens per chunk</dd></div>
        <div>
          <dt>Licence</dt>
          <dd>
            {#if current.licenceUrl}
              <button class="link" onclick={() => void openUrl(current.licenceUrl)}>{current.licence}</button>
            {:else}
              {current.licence}
            {/if}
          </dd>
        </div>
      {/if}
      {#if status?.readyModel && status.readyModel !== overview.model}
        <div><dt>Serving</dt><dd class="mono">{status.readyModel} <span class="muted">until the switch is done</span></dd></div>
      {/if}
    </dl>

    <h3>Models Hush knows</h3>
    <ul class="models">
      {#each overview.models as m (m.id)}
        {@const d = downloads[m.id]}
        <li>
          <div class="name">
            <strong>{m.name}</strong>
            <span class="muted mono">{m.id}</span>
          </div>
          <span class="muted" class:unavailable={m.availability.state !== "available" && (m.bundled || m.availability.state !== "missing")}>
            {availabilityText(m)} · {m.dims} dimensions · {m.licence}{#if m.rate != null}
              · {m.rate.toFixed(1)} notes/s here{/if}
          </span>
          {#if d?.state === "running"}
            <div class="progress" role="status">
              <span class="bar" aria-hidden="true"><span style:width="{pct(d.done, d.total)}%"></span></span>
              <span class="muted">
                {mb(d.done)} of {mb(d.total)} ({pct(d.done, d.total).toFixed(0)}%){#if d.bytesPerSec != null},
                  {mb(d.bytesPerSec)}/s{/if}
              </span>
            </div>
          {:else if d?.state === "cancelled"}
            <span class="muted">Download paused at {mb(d.done)} of {mb(d.total)}; it continues from there.</span>
          {:else if d?.state === "failed"}
            <span class="error">{d.message}</span>
          {/if}
          {#if modelErrors[m.id]}<span class="error" role="alert">{modelErrors[m.id]}</span>{/if}
          {#if notices[m.id]}<span class="muted">{notices[m.id]}</span>{/if}
          <div class="pick">
            {#if m.id === overview.model}
              <span class="muted">In use</span>
            {:else if m.availability.state === "available"}
              <button onclick={() => void use(m)} disabled={busy}>Use for this vault</button>
            {/if}
            {#if d?.state === "running"}
              <button onclick={() => void api.cancelModelDownload(m.id)}>Cancel</button>
            {:else if m.availability.state !== "available"}
              {#if !m.bundled}
                <button
                  onclick={() => void download(m)}
                  disabled={busy}
                  title="Downloads {mb(m.size)} from {m.host}. Hush goes online only for this."
                >
                  {d?.state === "cancelled" || d?.state === "failed" ? "Resume download" : "Download"}
                  ({mb(m.size)} from {m.host})
                </button>
              {/if}
              <button onclick={() => void importFiles(m)} disabled={busy} title="For computers that stay offline">
                Load model file…
              </button>
            {:else if m.availability.downloaded && !m.bundled && m.id !== overview.model}
              <button class="ghost" onclick={() => void remove(m)} disabled={busy}>Remove</button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="actions">
    <button onclick={() => void recheck()} disabled={busy} title="Check the model files again, and restart indexing if it stopped on them">
      Check again
    </button>
  </div>
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
    scroll-margin-top: 16px;
  }

  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  h3 {
    margin: 4px 0 0;
    font-size: 13px;
    font-weight: 600;
  }

  p {
    margin: 0;
    font-size: 13px;
  }

  .current {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 12px;
    margin: 0;
    font-size: 13px;
  }

  .current div {
    display: contents;
  }

  dt {
    color: var(--muted);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .mono {
    font-family: var(--font-mono, monospace);
    font-size: 12px;
  }

  .models {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 13px;
  }

  .models li {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .name {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
  }

  .models li > .muted,
  .models li > .error {
    font-size: 12px;
  }

  .unavailable {
    color: var(--danger, var(--muted));
  }

  .pick {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .progress {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }

  .bar {
    flex: 0 0 160px;
    height: 6px;
    border-radius: 3px;
    background: var(--border);
    overflow: hidden;
  }

  .bar > span {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }

  .link {
    font-size: inherit;
  }
</style>
