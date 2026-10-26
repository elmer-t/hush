<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    ACTIVITY_EVENT,
    api,
    errorMessage,
    type ActivityJob,
    type ActivitySnapshot,
    type ActivityState,
    type JobStatus,
  } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const JOB_NAMES: Record<ActivityJob, string> = {
    embedding: "Embedding",
    related: "Related index",
    training: "Tag training",
    safetyNet: "Safety-net scan",
    import: "Import",
    discover: "Discover tags",
    migration: "Migrations",
  };
  const STATE_NAMES: Record<ActivityState, string> = {
    idle: "idle",
    waiting: "waiting",
    running: "running",
    paused: "paused",
    stopped: "stopped",
    failed: "failed",
  };

  let snapshot = $state<ActivitySnapshot | null>(null);
  let error = $state("");
  let filter = $state<ActivityJob | "all">("all");
  let copied = $state(false);
  let now = $state(Date.now());

  const log = $derived(snapshot ? snapshot.log.filter((e) => filter === "all" || e.job === filter) : []);
  const stats = $derived(snapshot?.stats ?? null);

  onMount(() => {
    // Snapshots can arrive before the first status: keep the newest.
    const take = (s: ActivitySnapshot) => {
      if (!snapshot || s.revision >= snapshot.revision) snapshot = s;
    };
    const unlisten = listen<ActivitySnapshot>(ACTIVITY_EVENT, (e) => take(e.payload));
    api.activityStatus().then(take, (e) => (error = errorMessage(e)));
    // The counts are only refreshed while the panel is open.
    void api.setActivityPanelOpen(true).catch(() => {});
    const ticker = setInterval(() => (now = Date.now()), 1000);
    return () => {
      clearInterval(ticker);
      void api.setActivityPanelOpen(false).catch(() => {});
      void unlisten.then((stop) => stop());
    };
  });

  const n = (v: number | null | undefined) => (v == null ? "–" : v.toLocaleString());

  /** `42 s`, `3 min`, `2 h 5 min`. */
  function span(ms: number): string {
    const s = Math.max(0, Math.round(ms / 1000));
    if (s < 60) return `${s} s`;
    if (s < 3600) return `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`;
    return `${Math.floor(s / 3600)} h ${String(Math.floor((s % 3600) / 60)).padStart(2, "0")} min`;
  }

  function duration(ms: number): string {
    return ms < 1000 ? `${ms} ms` : ms < 60_000 ? `${(ms / 1000).toFixed(1)} s` : span(ms);
  }

  function ago(at: number): string {
    return `${span(now - at)} ago`;
  }

  function clock(at: number): string {
    return new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }

  function percent(job: JobStatus): number | null {
    if (job.done == null || !job.total) return null;
    return Math.min(100, (job.done / job.total) * 100);
  }

  function mb(bytes: number | null): string {
    return bytes == null ? "–" : `${(bytes / 1_048_576).toFixed(1)} MB`;
  }

  /** The log as plain text, for a bug report. Only what the panel shows. */
  async function copyLog() {
    const lines = log.map(
      (e) => `${new Date(e.at).toISOString()} ${JOB_NAMES[e.job]}${e.error ? " ERROR" : ""}: ${e.message}`,
    );
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch (e) {
      error = errorMessage(e);
    }
  }
</script>

<section class="activity" aria-label="Activity">
  <header>
    <Icon name="activity" size={15} />
    <h2>Activity</h2>
    <span class="kbd">Ctrl J</span>
    {#if error}<span class="error">{error}</span>{/if}
    <span class="spacer"></span>
    <button class="ghost icon" onclick={onclose} aria-label="Close activity panel" title="Close activity panel">
      <Icon name="close" />
    </button>
  </header>

  <div class="body">
    <div class="left">
      <table class="jobs">
        <thead>
          <tr>
            <th>Job</th>
            <th>State</th>
            <th>Now</th>
            <th class="num">Progress</th>
            <th class="num">Rate</th>
            <th class="num">ETA</th>
            <th>Last run</th>
          </tr>
        </thead>
        <tbody>
          {#each snapshot?.jobs ?? [] as job (job.job)}
            {@const p = percent(job)}
            <tr class:active={job.state === "running"}>
              <th scope="row">{JOB_NAMES[job.job]}</th>
              <td><span class="state {job.state}">{STATE_NAMES[job.state]}</span></td>
              <td class="now">
                {#if job.doing}<span>{job.doing}</span>{/if}
                {#if job.reason}<span class="reason" class:failed={job.state === "failed"}>{job.reason}</span>{/if}
                {#if job.state === "running" && job.startedAt}<span class="muted">for {span(now - job.startedAt)}</span>{/if}
              </td>
              <td class="num">
                {#if job.done != null}
                  {n(job.done)}{#if job.total != null}&thinsp;/&thinsp;{n(job.total)}{/if}
                  {#if p != null}
                    <span class="bar" aria-hidden="true"><span style:width="{p}%"></span></span>
                  {/if}
                {/if}
              </td>
              <td class="num">{job.rate != null ? `${job.rate.toFixed(1)}/s` : ""}</td>
              <td class="num">{job.etaSecs != null && job.state === "running" ? span(job.etaSecs * 1000) : ""}</td>
              <td class="last">
                {#if job.lastRun}
                  <span title={new Date(job.lastRun.finishedAt).toLocaleString()}>
                    {ago(job.lastRun.finishedAt)}, {duration(job.lastRun.durationMs)}
                  </span>
                  <span class="muted">{job.lastRun.summary}</span>
                {:else}
                  <span class="muted">not this session</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>

      <dl class="stats">
        <div><dt>Notes</dt><dd>{n(stats?.notes)}</dd></div>
        <div><dt>With vector</dt><dd>{n(stats?.withVector)}</dd></div>
        <div>
          <dt>Without</dt>
          <dd>{stats?.notes != null && stats?.withVector != null ? n(Math.max(0, stats.notes - stats.withVector)) : "–"}</dd>
        </div>
        <div><dt>Queued</dt><dd>{n(stats?.queued)}</dd></div>
        <div>
          <dt>Model</dt>
          <dd>
            {stats?.model ?? "–"}
            {#if stats?.model}
              <span class="muted">
                {stats.modelLoaded ? "loaded" : "not loaded"}, {stats.readyModel === stats.model ? "ready" : "not ready"}
              </span>
            {/if}
          </dd>
        </div>
        <div><dt>Cores</dt><dd>{n(stats?.coresUsed)} of {n(stats?.coresTotal)}</dd></div>
        <div><dt>Related index</dt><dd>{n(stats?.relatedNotes)} <span class="muted">{mb(stats?.relatedBytes ?? null)}</span></dd></div>
        <div>
          <dt>Tag heads</dt>
          <dd>
            {#if stats && stats.heads.length > 0}
              <details>
                <summary>{stats.heads.length.toLocaleString()}</summary>
                <ul>
                  {#each stats.heads as head (head.tag)}
                    <li title={`${head.model}, ${new Date(head.trainedAt).toLocaleString()}`}>
                      {head.tag} <span class="muted">{ago(head.trainedAt)}</span>
                    </li>
                  {/each}
                </ul>
              </details>
            {:else}
              {stats?.countedAt != null ? "0" : "–"}
            {/if}
          </dd>
        </div>
        <div><dt>Schema</dt><dd>{stats?.schemaVersion != null ? `v${stats.schemaVersion}` : "–"}</dd></div>
        <div>
          <dt>Counted</dt>
          <dd class="muted">{stats?.countedAt != null ? clock(stats.countedAt) : "–"}</dd>
        </div>
      </dl>
    </div>

    <div class="log">
      <div class="log-head">
        <h3>Event log</h3>
        <select bind:value={filter} aria-label="Show events of">
          <option value="all">All jobs</option>
          {#each Object.entries(JOB_NAMES) as [job, name] (job)}
            <option value={job}>{name}</option>
          {/each}
        </select>
        <button class="ghost" onclick={copyLog} disabled={log.length === 0} title="Copy the shown entries for a bug report">
          <Icon name={copied ? "check" : "copy"} size={14} />
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      {#if log.length === 0}
        <p class="muted empty">No events yet.</p>
      {:else}
        <ol>
          {#each log as entry (entry.seq)}
            <li class:error={entry.error}>
              <time datetime={new Date(entry.at).toISOString()}>{clock(entry.at)}</time>
              <span class="job">{JOB_NAMES[entry.job]}</span>
              <span class="message">{entry.message}</span>
            </li>
          {/each}
        </ol>
      {/if}
    </div>
  </div>
</section>

<style>
  .activity {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    border-top: 1px solid var(--border);
    background: var(--panel);
    font-size: 12px;
  }

  header {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 6px 2px 12px;
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
  }

  h2,
  h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }

  .kbd {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--muted);
  }

  .spacer {
    flex: 1;
  }

  .error,
  .failed {
    color: var(--warn);
  }

  .body {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 3fr) minmax(260px, 2fr);
  }

  .left {
    min-height: 0;
    overflow: auto;
    border-right: 1px solid var(--border);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-variant-numeric: tabular-nums;
  }

  th,
  td {
    padding: 3px 8px;
    text-align: left;
    vertical-align: top;
    border-bottom: 1px solid var(--border);
  }

  thead th {
    position: sticky;
    top: 0;
    background: var(--panel);
    font-weight: 500;
    color: var(--muted);
    white-space: nowrap;
  }

  tbody th {
    font-weight: 500;
    white-space: nowrap;
  }

  .num {
    text-align: right;
    white-space: nowrap;
  }

  .now span,
  .last span {
    display: block;
  }

  .reason {
    color: var(--text-2);
  }

  .state {
    display: inline-block;
    padding: 0 6px;
    border-radius: 4px;
    font-family: var(--mono);
    font-size: 11px;
    background: var(--seg);
    color: var(--text-2);
  }

  .state.running {
    background: var(--accent-soft);
    color: var(--accent-soft-text);
  }

  .state.waiting,
  .state.paused {
    background: var(--warn-soft);
    color: var(--warn);
  }

  .state.failed {
    background: var(--danger-soft);
    color: var(--danger);
  }

  .bar {
    display: block;
    height: 3px;
    margin-top: 2px;
    border-radius: 2px;
    background: var(--seg);
    overflow: hidden;
  }

  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .stats {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 18px;
    margin: 0;
    padding: 8px;
  }

  .stats div {
    display: flex;
    gap: 6px;
    align-items: baseline;
  }

  dt {
    color: var(--muted);
  }

  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }

  details ul {
    margin: 2px 0 0;
    padding-left: 16px;
    max-height: 120px;
    overflow: auto;
  }

  .log {
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .log-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 8px;
    border-bottom: 1px solid var(--border);
  }

  .log-head h3 {
    flex: 1;
    white-space: nowrap;
  }

  .log-head select,
  .log-head button {
    flex: none;
    width: auto;
    height: 24px;
    font-size: 12px;
  }

  ol {
    flex: 1;
    min-height: 0;
    overflow: auto;
    margin: 0;
    padding: 4px 0;
    list-style: none;
    font-family: var(--mono);
    font-size: 11px;
  }

  li {
    display: grid;
    grid-template-columns: auto 8.5em minmax(0, 1fr);
    gap: 8px;
    padding: 1px 8px;
  }

  li.error {
    color: var(--danger);
  }

  time,
  .job {
    color: var(--muted);
  }

  .empty {
    margin: 0;
    padding: 8px;
  }
</style>
