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
    type GpuStatus,
    type ModelInfo,
    type ModelOverview,
  } from "$lib/api";
  import { MODEL_FEATURES, model, reducedText } from "$lib/modelStatus.svelte";
  import { indexText, indexing } from "$lib/indexing";

  let overview = $state<ModelOverview | null>(null);
  let gpu = $state<GpuStatus | null>(null);
  /** Measured on an RTX 4060 Ti with 3,000-word notes (#105): the switch estimate until
   * this computer has a rate of its own. */
  const GPU_RATE = 15;
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
    api.gpuStatus().then((s) => (gpu = s), () => {});
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
    const progress = indexing(i);
    const indexingText = progress ? `indexing: ${indexText(progress)}` : null;
    if (!s || s.model !== overview.model) return indexingText ?? "not loaded yet";
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
    if (indexingText) return indexingText;
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
    const gpuGuess = m.rate == null && m.gpu && gpu?.best != null && gpu.enabled;
    const rate = m.rate ?? (gpuGuess ? GPU_RATE : overview.defaultRate);
    const measured =
      m.rate != null
        ? "measured on this computer"
        : gpuGuess
          ? "measured with long notes on an RTX 4060 Ti, not yet on this computer"
          : "an estimate, not yet measured on this computer";
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

<!-- Settings gives this its "Model" section header (id model-settings). -->
<div class="card model">
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
    <div class="in-use">
      <div class="in-use-head">
        <span class="led on" aria-hidden="true"></span>
        <span class="badge">In use</span>
        <strong>{current?.name ?? overview.model}</strong>
      </div>
      <dl class="current">
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
    </div>

    {#if gpu?.best && gpu.enabled && current && !current.gpu}
      {@const gpuModel = overview.models.find((m) => m.gpu)}
      {#if gpuModel}
        <p class="muted" role="status">
          This computer has a separate GPU ({gpu.best.name}). {gpuModel.name} runs on it: on an RTX 4060 Ti it indexed long
          notes about 25 times faster than the bundled model on the CPU. To use it, pick it in the list below
          (download it first if it isn't on this computer). Switching embeds every note again once.
        </p>
      {/if}
    {/if}

    <h3>Models Hush knows</h3>
    <ul class="models">
      {#each overview.models as m (m.id)}
        {@const d = downloads[m.id]}
        {@const inUse = m.id === overview.model}
        {@const serving = !inUse && status?.readyModel === m.id}
        <li class:selected={inUse} class:serving aria-current={inUse ? "true" : undefined}>
          <div class="name">
            <span class="led" class:on={inUse} class:half={serving} aria-hidden="true"></span>
            <strong>{m.name}</strong>
            {#if inUse}<span class="badge">In use</span>{:else if serving}<span class="badge outline">Serving until the switch is done</span>{/if}
            <span class="muted mono">{m.id}</span>
          </div>
          <span class="muted" class:unavailable={m.availability.state !== "available" && (m.bundled || m.availability.state !== "missing")}>
            {availabilityText(m)} · {m.dims} dimensions · {m.licence}{#if m.gpu}{" · made for the GPU"}{/if}{#if m.rate != null}{` · ${m.rate.toFixed(1)} notes/s here`}{/if}
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
            {#if !inUse && m.availability.state === "available"}
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
</div>

<style>
  .model {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
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

  .in-use {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border: 1px solid var(--accent);
    border-left-width: 4px;
    border-radius: 8px;
    background: var(--accent-soft);
    color: var(--accent-soft-text);
  }

  .in-use-head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    font-size: 15px;
  }

  .in-use dt {
    color: inherit;
    opacity: 0.75;
  }

  /* An LED: lit for the model in use, half-lit for the one serving during a switch. */
  .led {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid var(--border-strong);
    align-self: center;
  }

  .led.on {
    border-color: var(--accent);
    background: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 25%, transparent);
  }

  .led.half {
    border-color: var(--accent);
    background: linear-gradient(90deg, var(--accent) 50%, transparent 50%);
  }

  .badge {
    flex: none;
    padding: 1px 7px;
    border-radius: 4px;
    background: var(--accent);
    color: var(--accent-text);
    font-family: var(--mono);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    align-self: center;
  }

  .badge.outline {
    background: none;
    border: 1px solid var(--accent);
    color: var(--accent);
    text-transform: none;
    letter-spacing: 0;
    font-weight: 400;
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
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 13px;
  }

  .models li {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-left-width: 4px;
    border-radius: 8px;
  }

  .models li.selected {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .models li.selected strong {
    color: var(--accent-soft-text);
  }

  .models li.serving {
    border-left-color: var(--accent);
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

  .pick:empty {
    display: none;
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
