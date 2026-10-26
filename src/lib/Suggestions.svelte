<script lang="ts">
  import { onMount, tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { listen } from "@tauri-apps/api/event";
  import {
    api,
    errorMessage,
    TAGS_TRAINED_EVENT,
    type MetadataEditReport,
    type ReviewNote,
    type ReviewOrder,
    type SuggestionReview,
    type TagTraining,
  } from "$lib/api";
  import Icon from "$lib/Icon.svelte";
  import ModelNotice from "$lib/ModelNotice.svelte";
  import { reducedText } from "$lib/modelStatus.svelte";
  import { isToggle, selectionKeys } from "$lib/selection";

  type Props = {
    /** The tag under review; `null` until one is picked. */
    tag: string | null;
    /** The open note. */
    selectedId: number | null;
    /** Opens a note in the centre pane; this view stays. */
    onopen: (id: number) => void;
    /** Bulk accept (the parent flushes and reloads the open note). Resolves to the report, or null when it failed. */
    onaccept: (ids: number[], tag: string, sources: string[]) => Promise<MetadataEditReport | null>;
    /** Bulk reject. Resolves to whether it worked. */
    onreject: (ids: number[], tag: string) => Promise<boolean>;
  };
  let { tag = $bindable(), selectedId, onopen, onaccept, onreject }: Props = $props();

  const PAGE_SIZE = 100;
  // A training pass is asked for after this many decisions, so the order follows them.
  const RETRAIN_EVERY = 10;

  let tags = $state<TagTraining[]>([]);
  let order = $state<ReviewOrder>("needsReview");
  let review = $state<SuggestionReview | null>(null);
  let limit = $state(PAGE_SIZE);
  let error = $state("");
  let notice = $state("");
  let busy = $state(false);
  let retraining = $state(false);
  // Decisions since the tag was picked; a pass is asked for at every multiple of RETRAIN_EVERY.
  let decisions = 0;
  let generation = 0;
  let list = $state<HTMLElement>();
  // Where the mouse is over the list, and the row with keyboard focus: the row under the
  // mouse (else the focused one) is kept in place on screen when the list re-sorts.
  let pointer: { x: number; y: number } | null = null;
  let focused: number | null = null;

  // Notes picked for a bulk action. Frontend state, never stored.
  const picked = new SvelteSet<number>();
  let anchor: number | null = null;

  const notes = $derived(review?.notes ?? []);
  const training = $derived(tags.find((t) => t.tag === tag) ?? null);

  onMount(() => {
    void loadTags();
    const unlisten = listen(TAGS_TRAINED_EVENT, () => {
      void loadTags();
      // A pass asked for here reloads when it ends.
      if (!retraining) void load(true);
    });
    return () => void unlisten.then((stop) => stop());
  });

  // A new tag starts over: the default order is "Needs review" once it has a classifier.
  let shownTag: string | null = null;
  $effect(() => {
    if (tag === shownTag) return;
    shownTag = tag;
    picked.clear();
    anchor = null;
    decisions = 0;
    limit = PAGE_SIZE;
    notice = "";
    review = null;
    list?.scrollTo({ top: 0 });
    void start(tag);
  });

  async function start(t: string | null) {
    if (t === null) return;
    if (!tags.some((x) => x.tag === t)) await loadTags();
    if (t !== tag) return;
    order = tags.find((x) => x.tag === t)?.status === "trained" ? "needsReview" : "mostLikely";
    await load(false);
  }

  async function loadTags() {
    try {
      tags = await api.tagTraining();
    } catch (e) {
      error = errorMessage(e);
    }
  }

  function rowOf(target: EventTarget | Element | null): number | null {
    const id = (target as HTMLElement | null)?.closest?.<HTMLElement>("[data-id]")?.dataset.id;
    return id === undefined ? null : Number(id);
  }

  /** Follows the mouse over the list and the row with keyboard focus. */
  function trackCursor(el: HTMLElement) {
    const move = (e: PointerEvent) => (pointer = { x: e.clientX, y: e.clientY });
    const leave = () => (pointer = null);
    const focus = (e: FocusEvent) => (focused = rowOf(e.target));
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerleave", leave);
    el.addEventListener("focusin", focus);
    return () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerleave", leave);
      el.removeEventListener("focusin", focus);
    };
  }

  /** The row under the mouse now (rows may have moved under a still mouse), else the focused one. */
  function cursorRow(): number | null {
    if (pointer) {
      const id = rowOf(document.elementFromPoint(pointer.x, pointer.y));
      if (id !== null) return id;
    }
    return focused !== null && notes.some((n) => n.id === focused) ? focused : null;
  }

  function rowTop(id: number): number | null {
    const row = list?.querySelector<HTMLElement>(`[data-id="${id}"]`);
    return row ? row.getBoundingClientRect().top : null;
  }

  /** Reloads the list; with `keep`, the row under the cursor stays where it is on screen. */
  async function load(keep: boolean) {
    const t = tag;
    if (t === null) return;
    const run = ++generation;
    const keepId = keep ? cursorRow() : null;
    const before = keepId === null ? null : rowTop(keepId);
    try {
      const fresh = await api.reviewTagSuggestions(t, order, limit);
      if (run !== generation) return;
      review = fresh;
      error = "";
    } catch (e) {
      if (run === generation) error = errorMessage(e);
      return;
    }
    const shown = new Set(notes.map((n) => n.id));
    for (const id of [...picked]) if (!shown.has(id)) picked.delete(id);
    if (keepId !== null && before !== null) {
      await tick();
      const after = rowTop(keepId);
      if (after !== null && list) list.scrollTop += after - before;
    }
  }

  function setOrder(next: ReviewOrder) {
    if (next === order) return;
    order = next;
    picked.clear();
    anchor = null;
    list?.scrollTo({ top: 0 });
    void load(false);
  }

  function showMore() {
    limit += PAGE_SIZE;
    void load(false);
  }

  /** Plain click opens; Ctrl/Cmd+click toggles; Shift+click picks a range from the last click. */
  function pick(id: number, e: MouseEvent) {
    if (e.shiftKey && anchor !== null) {
      const ids = notes.map((n) => n.id);
      const from = ids.indexOf(anchor);
      const to = ids.indexOf(id);
      if (from >= 0 && to >= 0) {
        if (!isToggle(e)) picked.clear();
        for (const i of ids.slice(Math.min(from, to), Math.max(from, to) + 1)) picked.add(i);
        return;
      }
    }
    if (isToggle(e)) {
      if (picked.has(id)) picked.delete(id);
      else picked.add(id);
      anchor = id;
      return;
    }
    picked.clear();
    anchor = id;
    onopen(id);
  }

  function selectAll() {
    for (const n of notes) picked.add(n.id);
  }

  function clearSelection() {
    picked.clear();
    anchor = null;
  }

  async function decide(ids: number[], accept: boolean) {
    const t = tag;
    if (busy || t === null || ids.length === 0) return;
    busy = true;
    notice = "";
    let decided = ids;
    if (accept) {
      const sources = notes.filter((n) => ids.includes(n.id)).map((n) => n.folder);
      const report = await onaccept(ids, t, sources);
      if (!report) {
        busy = false;
        return;
      }
      const skipped = new Set(report.unreadable.map((n) => n.id));
      decided = ids.filter((id) => !skipped.has(id));
      if (skipped.size > 0) {
        const n = skipped.size;
        notice = `${n} ${n === 1 ? "note" : "notes"} skipped: frontmatter can't be read.`;
      }
    } else if (!(await onreject(ids, t))) {
      busy = false;
      return;
    }
    busy = false;
    if (t !== tag || !review) return;
    const gone = new Set(decided);
    review = { ...review, notes: review.notes.filter((n) => !gone.has(n.id)), total: review.total - gone.size };
    for (const id of decided) picked.delete(id);
    if (focused !== null && gone.has(focused)) focused = null;
    const before = decisions;
    decisions += decided.length;
    if (Math.floor(decisions / RETRAIN_EVERY) > Math.floor(before / RETRAIN_EVERY)) void retrain();
  }

  /** Trains now rather than after the job's quiet period, then re-sorts. */
  async function retrain() {
    if (retraining) return;
    retraining = true;
    try {
      await api.retrainTags();
    } catch (e) {
      error = errorMessage(e);
    }
    retraining = false;
    await Promise.all([loadTags(), load(true)]);
  }

  function score(n: ReviewNote): string {
    return review?.method === "classifier" ? `${Math.round(n.score * 100)}%` : n.score.toFixed(2);
  }

  function status(t: TagTraining): string {
    switch (t.status) {
      case "trained":
        return "classifier";
      case "waiting":
        return "training soon";
      case "descriptionOnly":
        return "description";
      case "needsExamples":
        return "can't be suggested yet";
    }
  }

  const summary = $derived.by(() => {
    if (!review || review.method === null) return "";
    const n = review.total;
    const what = n === 1 ? "1 note" : `${n.toLocaleString()} notes`;
    const how =
      review.method === "classifier"
        ? `classifier, from ${Math.round(review.minScore * 100)}%`
        : `description, similarity from ${review.minScore.toFixed(2)}`;
    return `${what} · ${how}`;
  });
</script>

<div class="suggestions">
  <div class="head">
    <select
      aria-label="Tag to review"
      value={tag ?? ""}
      onchange={(e) => (tag = e.currentTarget.value || null)}
    >
      <option value="" disabled>Pick a tag to review</option>
      {#each tags as t (t.tag)}
        <option value={t.tag}>#{t.tag} · {status(t)}</option>
      {/each}
    </select>
    {#if tag !== null}
      <div class="bar">
        <div class="segmented" role="group" aria-label="Order">
          <button
            aria-pressed={order === "needsReview"}
            onclick={() => setOrder("needsReview")}
            title="Least sure first: answering these teaches the classifier fastest"
          >
            Needs review
          </button>
          <button
            aria-pressed={order === "mostLikely"}
            onclick={() => setOrder("mostLikely")}
            title="Most likely first, to tag many notes at once"
          >
            Most likely
          </button>
        </div>
        {#if retraining}<span class="muted training" role="status">Retraining…</span>{/if}
      </div>
      {#if summary}<p class="summary">{summary}</p>{/if}
    {/if}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if notice}<p class="notice" role="status">{notice}</p>{/if}

  <div
    class="list"
    bind:this={list}
    {@attach trackCursor}
    {@attach selectionKeys(selectAll, clearSelection)}
  >
    {#if tag === null}
      <p class="muted empty">
        Pick a tag to see the notes it would be suggested on, across the vault. Answering the ones Hush is least sure
        about teaches it fastest.
      </p>
    {:else if notes.length === 0 && reducedText()}
      <div class="empty"><ModelNotice what="Tag suggestions" /></div>
    {:else if review && review.method === null}
      <p class="muted empty">
        #{tag} can't be suggested yet: describe it in Settings → Tags{#if training && training.needed > 0}, or add it
          to {training.needed} more {training.needed === 1 ? "note" : "notes"} so Hush can train a classifier{/if}.
      </p>
    {:else if review && notes.length === 0}
      <p class="muted empty">No suggestions for #{tag} right now.</p>
    {/if}
    {#each notes as n (n.id)}
      <div
        class="row"
        class:selected={n.id === selectedId}
        class:picked={picked.has(n.id)}
        data-id={n.id}
      >
        <button
          class="open"
          aria-current={n.id === selectedId}
          aria-pressed={picked.size > 0 ? picked.has(n.id) : undefined}
          onclick={(e) => pick(n.id, e)}
        >
          {#if picked.size > 0}
            <span class="check" class:on={picked.has(n.id)} aria-hidden="true">
              {#if picked.has(n.id)}<Icon name="check" size={12} strokeWidth={3} />{/if}
            </span>
          {/if}
          <span class="text">
            <span class="title-line">
              <span class="title">{n.title}</span>
              <span class="score" title={review?.method === "classifier" ? "Probability" : "Similarity"}>{score(n)}</span>
            </span>
            <span class="folder">{n.folder || "vault root"}</span>
            {#if n.excerpt}<span class="excerpt">{n.excerpt}</span>{/if}
          </span>
        </button>
        <div class="decide">
          <button
            class="small icon accept"
            aria-label="Accept #{tag} on {n.title}"
            title="Accept"
            disabled={busy}
            onclick={() => void decide([n.id], true)}
          >
            <Icon name="check" size={14} strokeWidth={2.5} />
          </button>
          <button
            class="small icon reject"
            aria-label="Reject #{tag} on {n.title}"
            title="Reject"
            disabled={busy}
            onclick={() => void decide([n.id], false)}
          >
            <Icon name="close" size={14} strokeWidth={2.5} />
          </button>
        </div>
      </div>
    {/each}
    {#if review && notes.length < review.total}
      <button class="small more" onclick={showMore}>
        Show more ({(review.total - notes.length).toLocaleString()} left)
      </button>
    {/if}
  </div>

  {#if picked.size > 0}
    <div class="actions" role="toolbar" aria-label="Selected suggestions">
      <span class="picked">{picked.size} selected</span>
      <button onclick={() => void decide([...picked], true)} disabled={busy} title="Add #{tag} to the selected notes">
        Accept
      </button>
      <button onclick={() => void decide([...picked], false)} disabled={busy} title="Don't suggest #{tag} on them again">
        Reject
      </button>
      <button class="esc" onclick={clearSelection} disabled={busy} aria-label="Clear selection (Esc)">Esc</button>
    </div>
  {/if}
</div>

<style>
  .suggestions {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .head {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 0 12px 8px;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .training {
    font-size: 12px;
  }

  .summary,
  .notice,
  .error {
    margin: 0;
    font-size: 12px;
  }

  .summary {
    color: var(--muted);
  }

  .notice,
  .error {
    padding: 0 12px 8px;
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 8px 8px;
  }

  .empty {
    margin: 0;
    padding: 4px 8px;
    font-size: 13px;
  }

  .row {
    display: flex;
    align-items: flex-start;
    gap: 4px;
    border: 1px solid transparent;
    border-radius: 8px;
  }

  .row:hover {
    background: var(--hover);
  }

  .row.selected {
    border-color: var(--accent);
  }

  .row.picked {
    background: var(--selection);
  }

  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-start;
    justify-content: flex-start;
    gap: 10px;
    height: auto;
    padding: 8px 4px 8px 10px;
    border: none;
    background: none;
    color: var(--text);
    text-align: left;
    white-space: normal;
  }

  .open:hover:not(:disabled) {
    background: none;
  }

  .check {
    flex: none;
    width: 16px;
    height: 16px;
    margin-top: 2px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border-strong);
    border-radius: 4px;
  }

  .check.on {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--accent-text);
  }

  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .title-line {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .title,
  .folder {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .title {
    flex: 1;
    min-width: 0;
    font-weight: 500;
  }

  .score {
    flex: none;
    font-size: 12px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }

  .folder {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--muted);
  }

  .excerpt {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    font-size: 13px;
    line-height: 1.35;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }

  .decide {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 6px 8px 0;
  }

  .decide button {
    color: var(--text-2);
    background: var(--surface);
  }

  .decide .accept:hover:not(:disabled) {
    color: var(--accent);
    border-color: var(--accent);
  }

  .decide .reject:hover:not(:disabled) {
    color: var(--danger);
  }

  .more {
    width: 100%;
    margin-top: 6px;
  }

  /* Inverted like the note list's selection bar, so it reads as a mode. */
  .actions {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 8px;
    padding: 8px 8px 8px 14px;
    border-radius: 10px;
    background: var(--text);
    color: var(--bg);
  }

  .picked {
    flex: 1;
    font-weight: 500;
  }

  .actions button {
    gap: 6px;
    padding: 0 10px;
    border: none;
    border-radius: 6px;
    background: var(--inverse-button);
    color: var(--bg);
    font-size: 13px;
  }

  .actions button:hover:not(:disabled) {
    filter: brightness(1.15);
  }

  .actions .esc {
    padding: 0 8px;
    background: transparent;
    font-family: var(--mono);
    font-size: 11px;
    opacity: 0.85;
  }
</style>
