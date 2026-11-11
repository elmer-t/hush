<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, errorMessage, GPU_STATUS_EVENT, type GpuStatus } from "$lib/api";
  import { gpuText } from "$lib/gpu";
  import ModelSettings from "$lib/ModelSettings.svelte";
  import PassphraseStrength from "$lib/PassphraseStrength.svelte";
  import TagSettings from "$lib/TagSettings.svelte";

  type Props = {
    /** Opens the Suggestions view for a tag. */
    onreviewtag: (tag: string) => void;
  };
  let { onreviewtag }: Props = $props();

  let minutes = $state(10);
  let minutesStatus = $state("");
  let minutesError = $state("");

  let current = $state("");
  let next = $state("");
  let confirm = $state("");
  let changing = $state(false);
  let passphraseStatus = $state("");
  let passphraseError = $state("");

  let confirmTrash = $state(true);
  let allCores = $state(false);
  let coresError = $state("");
  let gpu = $state<GpuStatus | null>(null);
  let useGpu = $state(true);
  let gpuError = $state("");
  let confirmError = $state("");

  let trashDays = $state(30);
  let trashStatus = $state("");
  let trashError = $state("");

  let recentCount = $state(10);
  let recentStatus = $state("");
  let recentError = $state("");

  let author = $state("");
  let authorStatus = $state("");
  let authorError = $state("");

  const SECTIONS = [
    { id: "settings-general", label: "General" },
    { id: "settings-security", label: "Security" },
    { id: "settings-tags", label: "Tags" },
    { id: "model-settings", label: "Model" },
    { id: "settings-indexing", label: "Indexing" },
    { id: "settings-trash", label: "Trash" },
  ];
  let root = $state<HTMLElement>();
  let active = $state(SECTIONS[0].id);
  /** Set while a menu click scrolls the page, so the sections it passes don't light up. */
  let jumping: string | null = null;

  /** The element that scrolls the settings: the workspace page, or the document. */
  function scroller(el: HTMLElement): HTMLElement {
    for (let p = el.parentElement; p; p = p.parentElement) {
      const y = getComputedStyle(p).overflowY;
      if (y === "auto" || y === "scroll") return p;
    }
    return document.documentElement;
  }

  /** The last section whose header has scrolled past the top; the last one at the bottom. */
  function track(box: HTMLElement) {
    if (jumping) return;
    const top = box === document.documentElement ? 0 : box.getBoundingClientRect().top;
    // Below the stacked menu on a narrow page, near the top otherwise.
    const line = top + (root?.querySelector(".menu")?.getBoundingClientRect().height ?? 0) + 24;
    let found = SECTIONS[0].id;
    for (const s of SECTIONS) {
      const el = document.getElementById(s.id);
      if (el && el.getBoundingClientRect().top <= Math.max(line, top + 96)) found = s.id;
    }
    if (box.scrollTop > 0 && box.scrollTop + box.clientHeight >= box.scrollHeight - 2) {
      found = SECTIONS[SECTIONS.length - 1].id;
    }
    active = found;
  }

  function jump(e: MouseEvent, id: string) {
    // The hash would go to the SvelteKit router.
    e.preventDefault();
    const el = document.getElementById(id);
    if (!el) return;
    active = jumping = id;
    const menu = root?.querySelector(".menu");
    if (menu && root && menu.getBoundingClientRect().width >= root.getBoundingClientRect().width - 1) {
      // Narrow page: the menu is a bar across the top, so stop below it.
      const box = scroller(root);
      const boxTop = box === document.documentElement ? 0 : box.getBoundingClientRect().top;
      const below = menu.getBoundingClientRect().bottom - boxTop;
      box.scrollTo({ top: box.scrollTop + el.getBoundingClientRect().top - boxTop - below - 12, behavior: "smooth" });
    } else {
      el.scrollIntoView({ block: "start", behavior: "smooth" });
    }
    el.querySelector<HTMLElement>("h2")?.focus({ preventScroll: true });
  }

  onMount(() => {
    if (!root) return;
    const box = scroller(root);
    const target: HTMLElement | Window = box === document.documentElement ? window : box;
    let frame = 0;
    const onScroll = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => track(box));
    };
    // A user scroll (wheel, keys, scrollbar) after a jump ends it.
    const onScrollEnd = () => {
      jumping = null;
    };
    const release = () => {
      jumping = null;
    };
    target.addEventListener("scroll", onScroll, { passive: true });
    target.addEventListener("scrollend", onScrollEnd);
    box.addEventListener("wheel", release, { passive: true });
    box.addEventListener("keydown", release);
    track(box);
    return () => {
      cancelAnimationFrame(frame);
      target.removeEventListener("scroll", onScroll);
      target.removeEventListener("scrollend", onScrollEnd);
      box.removeEventListener("wheel", release);
      box.removeEventListener("keydown", release);
    };
  });

  onMount(() => {
    const unlisten = listen<GpuStatus>(GPU_STATUS_EVENT, (e) => (gpu = e.payload));
    return () => void unlisten.then((stop) => stop());
  });

  onMount(async () => {
    api.indexAllCores().then((on) => (allCores = on), () => {});
    api.gpuStatus().then((s) => ((gpu = s), (useGpu = s.enabled)), () => {});
    [minutes, author, trashDays, confirmTrash, recentCount] = await Promise.all([
      api.autoLockMinutes(),
      api.defaultAuthor().then((a) => a ?? ""),
      api.trashDays(),
      api.confirmTrash(),
      api.recentOpenedCount(),
    ]);
  });

  async function saveAllCores() {
    coresError = "";
    try {
      await api.setIndexAllCores(allCores);
    } catch (err) {
      coresError = errorMessage(err);
      allCores = !allCores;
    }
  }

  async function saveUseGpu() {
    gpuError = "";
    try {
      await api.setIndexGpu(useGpu);
      gpu = await api.gpuStatus();
    } catch (err) {
      gpuError = errorMessage(err);
      useGpu = !useGpu;
    }
  }

  // Saved on change: a checkbox has no Save button.
  async function saveConfirmTrash() {
    confirmError = "";
    try {
      await api.setConfirmTrash(confirmTrash);
    } catch (err) {
      confirmError = errorMessage(err);
      confirmTrash = !confirmTrash;
    }
  }

  async function saveTrashDays(e: SubmitEvent) {
    e.preventDefault();
    trashStatus = trashError = "";
    try {
      await api.setTrashDays(trashDays);
      trashStatus = "Saved.";
    } catch (err) {
      trashError = errorMessage(err);
    }
  }

  async function saveRecentCount(e: SubmitEvent) {
    e.preventDefault();
    recentStatus = recentError = "";
    try {
      await api.setRecentOpenedCount(recentCount);
      recentStatus = "Saved.";
    } catch (err) {
      recentError = errorMessage(err);
    }
  }

  async function clearRecent() {
    recentStatus = recentError = "";
    try {
      await api.clearRecentOpened();
      recentStatus = "List cleared.";
    } catch (err) {
      recentError = errorMessage(err);
    }
  }

  async function saveAuthor(e: SubmitEvent) {
    e.preventDefault();
    authorStatus = authorError = "";
    try {
      await api.setDefaultAuthor(author);
      authorStatus = "Saved.";
    } catch (err) {
      authorError = errorMessage(err);
    }
  }

  async function saveMinutes(e: SubmitEvent) {
    e.preventDefault();
    minutesStatus = minutesError = "";
    try {
      await api.setAutoLockMinutes(minutes);
      minutesStatus = "Saved.";
    } catch (err) {
      minutesError = errorMessage(err);
    }
  }

  async function changePassphrase(e: SubmitEvent) {
    e.preventDefault();
    passphraseStatus = passphraseError = "";
    if (next !== confirm) {
      passphraseError = "The new passphrases do not match.";
      return;
    }
    changing = true;
    try {
      await api.changePassphrase(current, next);
      passphraseStatus = "Passphrase changed.";
      current = next = confirm = "";
    } catch (err) {
      passphraseError = errorMessage(err);
    } finally {
      changing = false;
    }
  }
</script>

<div class="frame" bind:this={root}>
<div class="settings">
  <nav class="menu" aria-label="Settings sections">
    <h1>Settings</h1>
    <ul>
      {#each SECTIONS as s, i (s.id)}
        <li>
          <a href="#{s.id}" aria-current={active === s.id ? "true" : undefined} onclick={(e) => jump(e, s.id)}>
            <span class="num" aria-hidden="true">{String(i + 1).padStart(2, "0")}</span>
            {s.label}
          </a>
        </li>
      {/each}
    </ul>
  </nav>

  <div class="sections">
    <section id="settings-general" aria-labelledby="settings-general-h">
      <h2 id="settings-general-h" tabindex="-1"><span class="num" aria-hidden="true">01</span>General</h2>
      <form class="card" onsubmit={saveAuthor}>
        <h3>Default author</h3>
        <label>
          Added to the frontmatter of new notes
          <input bind:value={author} placeholder="No default author" />
        </label>
        {#if authorError}<p class="error" role="alert">{authorError}</p>{/if}
        <div class="actions">
          {#if authorStatus}<span class="muted">{authorStatus}</span>{/if}
          <button type="submit">Save</button>
        </div>
      </form>

      <form class="card" onsubmit={saveRecentCount}>
        <h3>Recently opened notes</h3>
        <label>
          How many notes the "Recently opened" list keeps (0 = off)
          <input type="number" min="0" max="1000" step="1" bind:value={recentCount} required />
        </label>
        <p class="muted">Kept inside the vault, per vault. Turning it off clears the list.</p>
        {#if recentError}<p class="error" role="alert">{recentError}</p>{/if}
        <div class="actions">
          {#if recentStatus}<span class="muted">{recentStatus}</span>{/if}
          <button type="button" onclick={clearRecent}>Clear list</button>
          <button type="submit">Save</button>
        </div>
      </form>
    </section>

    <section id="settings-security" aria-labelledby="settings-security-h">
      <h2 id="settings-security-h" tabindex="-1"><span class="num" aria-hidden="true">02</span>Security</h2>
      <form class="card" onsubmit={saveMinutes}>
        <h3>Auto-lock</h3>
        <label>
          Lock after this many idle minutes
          <input type="number" min="1" max="1440" step="1" bind:value={minutes} required />
        </label>
        {#if minutesError}<p class="error" role="alert">{minutesError}</p>{/if}
        <div class="actions">
          {#if minutesStatus}<span class="muted">{minutesStatus}</span>{/if}
          <button type="submit">Save</button>
        </div>
      </form>

      <form class="card" onsubmit={changePassphrase}>
        <h3>Change passphrase</h3>
        <label>
          Current passphrase
          <input type="password" autocomplete="off" bind:value={current} disabled={changing} required />
        </label>
        <div class="new-passphrase">
          <label>
            New passphrase
            <input
              type="password"
              autocomplete="off"
              aria-describedby="new-passphrase-strength"
              bind:value={next}
              disabled={changing}
              required
            />
          </label>
          <PassphraseStrength passphrase={next} id="new-passphrase-strength" />
        </div>
        <label>
          Repeat new passphrase
          <input type="password" autocomplete="off" bind:value={confirm} disabled={changing} required />
        </label>
        <p class="muted">There is no recovery. Make sure you can remember the new passphrase.</p>
        {#if passphraseError}<p class="error" role="alert">{passphraseError}</p>{/if}
        <div class="actions">
          {#if passphraseStatus}<span class="muted">{passphraseStatus}</span>{/if}
          <button type="submit" class="primary" disabled={changing}>
            {changing ? "Changing…" : "Change passphrase"}
          </button>
        </div>
      </form>
    </section>

    <section id="settings-tags" aria-labelledby="settings-tags-h">
      <h2 id="settings-tags-h" tabindex="-1"><span class="num" aria-hidden="true">03</span>Tags</h2>
      <TagSettings onreview={onreviewtag} />
    </section>

    <!-- The id is what "why is this off" links scroll to (Workspace.openModelSettings). -->
    <section id="model-settings" aria-labelledby="model-settings-h">
      <h2 id="model-settings-h" tabindex="-1"><span class="num" aria-hidden="true">04</span>Model</h2>
      <ModelSettings />
    </section>

    <section id="settings-indexing" aria-labelledby="settings-indexing-h">
      <h2 id="settings-indexing-h" tabindex="-1"><span class="num" aria-hidden="true">05</span>Indexing</h2>
      <div class="card">
        <p class="muted">
          Hush works out what each note is about on this computer, in the background. Indexing pauses while the vault
          is locked and picks up where it left off, most recently changed notes first.
        </p>
        <label class="check">
          <input type="checkbox" bind:checked={allCores} onchange={saveAllCores} />
          Use all cores while indexing (faster, but the computer may get busy and warm)
        </label>
        {#if coresError}<p class="error" role="alert">{coresError}</p>{/if}
        <label class="check">
          <input type="checkbox" bind:checked={useGpu} onchange={saveUseGpu} />
          Use the GPU for models made for it (Windows, through DirectML)
        </label>
        {#if gpu}<p class="muted" role="status">{gpuText(gpu)}</p>{/if}
        {#if gpuError}<p class="error" role="alert">{gpuError}</p>{/if}
      </div>
    </section>

    <section id="settings-trash" aria-labelledby="settings-trash-h">
      <h2 id="settings-trash-h" tabindex="-1"><span class="num" aria-hidden="true">06</span>Trash</h2>
      <form class="card" onsubmit={saveTrashDays}>
        <label>
          Delete notes in the trash forever after this many days (0 = never)
          <input type="number" min="0" max="3650" step="1" bind:value={trashDays} required />
        </label>
        <p class="muted">Checked each time the vault is unlocked.</p>
        <label class="check">
          <input type="checkbox" bind:checked={confirmTrash} onchange={saveConfirmTrash} />
          Confirm before moving notes to the trash
        </label>
        {#if confirmError}<p class="error" role="alert">{confirmError}</p>{/if}
        {#if trashError}<p class="error" role="alert">{trashError}</p>{/if}
        <div class="actions">
          {#if trashStatus}<span class="muted">{trashStatus}</span>{/if}
          <button type="submit">Save</button>
        </div>
      </form>
    </section>
  </div>
</div>
</div>

<style>
  /* The menu stacks above the sections when the page is narrow (sidebars open). */
  .frame {
    container-type: inline-size;
  }

  .settings {
    max-width: 820px;
    margin: 0 auto;
    display: grid;
    grid-template-columns: 168px minmax(0, 1fr);
    gap: 32px;
    align-items: start;
  }

  .menu {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .menu ul {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .menu a {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 6px 10px;
    border-left: 2px solid transparent;
    border-radius: 0 6px 6px 0;
    color: var(--text-2);
    font-size: 13px;
    text-decoration: none;
  }

  .menu a:hover {
    background: var(--hover);
    color: var(--text);
  }

  .menu a[aria-current="true"] {
    border-left-color: var(--accent);
    background: var(--accent-soft);
    color: var(--accent-soft-text);
    font-weight: 600;
  }

  .num {
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 400;
    color: var(--muted);
  }

  .menu a[aria-current="true"] .num {
    color: inherit;
  }

  .sections {
    display: flex;
    flex-direction: column;
    gap: 36px;
    min-width: 0;
  }

  .sections > section {
    display: flex;
    flex-direction: column;
    gap: 14px;
    scroll-margin-top: 16px;
  }

  h2 {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 0;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border-strong);
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  h2:focus {
    outline: none;
  }

  h2 .num {
    letter-spacing: 0;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 20px;
  }

  h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  label > input {
    min-height: 36px;
    border-radius: 8px;
  }

  .new-passphrase {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .check {
    flex-direction: row;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    color: var(--text);
  }

  p {
    margin: 0;
    font-size: 13px;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
  }

  @container (max-width: 640px) {
    .settings {
      grid-template-columns: minmax(0, 1fr);
      gap: 20px;
    }

    .menu {
      /* Flush with the top of the workspace page, whose padding is 32px. */
      top: -32px;
      z-index: 1;
      margin: -32px -16px 0;
      padding: 16px 16px 8px;
      background: var(--bg);
      border-bottom: 1px solid var(--border);
    }

    .menu ul {
      flex-direction: row;
      flex-wrap: wrap;
      gap: 4px;
    }

    .menu a {
      border-left: 0;
      border-bottom: 2px solid transparent;
      border-radius: 6px 6px 0 0;
    }

    .menu a[aria-current="true"] {
      border-bottom-color: var(--accent);
    }

    /* Below the menu bar when something else scrolls a section into view. */
    .sections > section {
      scroll-margin-top: 140px;
    }
  }
</style>
