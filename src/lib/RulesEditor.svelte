<script lang="ts">
  import { onMount } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api, errorMessage, type Rule, type RuleOutput, type RuleSource } from "$lib/api";
  import { longDate } from "$lib/dates";

  const SOURCES: [RuleSource, string][] = [
    ["fileName", "File name"],
    ["frontmatter", "Frontmatter"],
    ["firstLines", "First lines"],
    ["body", "Whole body"],
  ];
  const TARGETS: [string, string][] = [
    ["tag", "Tag"],
    ["author", "Author"],
    ["author_email", "Author email"],
    ["topic", "Topic"],
    ["created_at", "Created date"],
    ["attribute", "Attribute…"],
  ];

  let rules = $state<Rule[]>([]);
  let loaded = $state(false);
  let dirty = $state(false);
  let saving = $state(false);
  let status = $state("");
  let error = $state("");

  let sampleName = $state("2004-03-12 jr Morning walk");
  let sampleText = $state("From: Jan de Vries <jan@example.org>\nDate: 2004-03-12\nSubject: Morning walk\n\nBody text…");
  // Which rule the result belongs to; -1 for all rules.
  let testedIndex = $state<number | null>(null);
  let result = $state<RuleOutput | null>(null);
  let testError = $state("");

  onMount(async () => {
    try {
      rules = await api.importRules();
      loaded = true;
    } catch (e) {
      error = errorMessage(e);
    }
  });

  function changed() {
    dirty = true;
    status = "";
    result = null;
  }

  function targetKind(rule: Rule) {
    return rule.target.startsWith("attribute:") ? "attribute" : rule.target;
  }

  function setTargetKind(rule: Rule, kind: string) {
    rule.target = kind === "attribute" ? `attribute:${attributeKey(rule)}` : kind;
    changed();
  }

  function attributeKey(rule: Rule) {
    return rule.target.startsWith("attribute:") ? rule.target.slice("attribute:".length) : "";
  }

  function add() {
    rules.push({
      name: "New rule",
      enabled: true,
      source: "firstLines",
      lines: 10,
      pattern: "",
      target: "tag",
      map: [],
      strip: false,
    });
    changed();
  }

  function move(i: number, by: number) {
    const [rule] = rules.splice(i, 1);
    rules.splice(i + by, 0, rule);
    changed();
  }

  function remove(i: number) {
    rules.splice(i, 1);
    changed();
  }

  async function resetDefaults() {
    const confirmed = await ask("Replace all rules with the defaults? Your rules are only replaced once you save.", {
      title: "Reset rules",
      kind: "warning",
      okLabel: "Reset",
      cancelLabel: "Cancel",
    });
    if (!confirmed) return;
    rules = await api.defaultImportRules();
    changed();
  }

  async function save() {
    saving = true;
    error = status = "";
    try {
      await api.setImportRules($state.snapshot(rules));
      dirty = false;
      status = "Saved.";
    } catch (e) {
      error = errorMessage(e);
    } finally {
      saving = false;
    }
  }

  async function test(index: number) {
    testError = "";
    result = null;
    testedIndex = index;
    const selected = index < 0 ? rules : [{ ...rules[index], enabled: true }];
    try {
      result = await api.testImportRules($state.snapshot(selected), sampleName, sampleText);
    } catch (e) {
      testError = errorMessage(e);
    }
  }
</script>

{#snippet testResult()}
  {#if testError}
    <p class="error" role="alert">{testError}</p>
  {:else if result}
    {@const r = result}
    <div class="result">
      <dl>
        {#if r.tags.length}<dt>Tags</dt><dd>{r.tags.map((t) => `#${t}`).join(" ")}</dd>{/if}
        {#if r.author}<dt>Author</dt><dd>{r.author}</dd>{/if}
        {#if r.authorEmail}<dt>Author email</dt><dd>{r.authorEmail}</dd>{/if}
        {#if r.topic}<dt>Topic</dt><dd>{r.topic}</dd>{/if}
        {#if r.createdAt !== null}<dt>Created</dt><dd>{longDate(r.createdAt)}</dd>{/if}
        {#each r.attributes as [key, values] (key)}<dt>{key}</dt><dd>{values.join(", ")}</dd>{/each}
        <dt>Title</dt>
        <dd>{r.fileName || sampleName}</dd>
      </dl>
      {#if !r.tags.length && !r.author && !r.authorEmail && !r.topic && r.createdAt === null && !r.attributes.length}
        <p class="muted">No matches.</p>
      {/if}
      {#each r.warnings as w, i (i)}<p class="error">{w}</p>{/each}
      <details>
        <summary>Body after stripping</summary>
        <pre>{r.content}</pre>
      </details>
    </div>
  {/if}
{/snippet}

<div class="rules">
  <p class="muted">
    Rules run on each file during import, in this order. Patterns use Rust regex syntax (no lookaround); <code>^</code>
    and <code>$</code> match at line breaks, and the first group, if any, is the value. For author, email, topic and date
    the first match wins; tags and attributes collect every match. Results are written into the note's frontmatter.
  </p>

  <section class="sample">
    <h2>Test sample</h2>
    <label>
      File name (without extension)
      <input bind:value={sampleName} spellcheck="false" />
    </label>
    <label>
      Text
      <textarea rows="6" bind:value={sampleText} spellcheck="false"></textarea>
    </label>
    <div class="actions">
      <button onclick={() => test(-1)} disabled={!loaded}>Test all rules</button>
    </div>
    {#if testedIndex === -1}{@render testResult()}{/if}
  </section>

  {#each rules as rule, i (i)}
    <section class="rule" class:disabled={!rule.enabled}>
      <div class="head">
        <input type="checkbox" bind:checked={rule.enabled} onchange={changed} aria-label="Enabled" />
        <input class="name" bind:value={rule.name} oninput={changed} aria-label="Rule name" />
        <button onclick={() => move(i, -1)} disabled={i === 0} aria-label="Move up">↑</button>
        <button onclick={() => move(i, 1)} disabled={i === rules.length - 1} aria-label="Move down">↓</button>
        <button onclick={() => remove(i)} aria-label="Delete rule {rule.name}">Delete</button>
      </div>
      <div class="grid">
        <label>
          Look in
          <select bind:value={rule.source} onchange={changed}>
            {#each SOURCES as [value, label] (value)}<option {value}>{label}</option>{/each}
          </select>
        </label>
        {#if rule.source === "firstLines"}
          <label>
            Lines
            <input type="number" min="1" max="1000" bind:value={rule.lines} oninput={changed} />
          </label>
        {/if}
        <label class="wide">
          Pattern
          <input class="mono" bind:value={rule.pattern} oninput={changed} spellcheck="false" />
        </label>
        <label>
          Sets
          <select value={targetKind(rule)} onchange={(e) => setTargetKind(rule, e.currentTarget.value)}>
            {#each TARGETS as [value, label] (value)}<option {value}>{label}</option>{/each}
          </select>
        </label>
        {#if targetKind(rule) === "attribute"}
          <label>
            Attribute name
            <input
              value={attributeKey(rule)}
              oninput={(e) => {
                rule.target = `attribute:${e.currentTarget.value.trim().toLowerCase()}`;
                changed();
              }}
            />
          </label>
        {/if}
        <label class="check">
          <input type="checkbox" bind:checked={rule.strip} onchange={changed} />
          Remove the match from the {rule.source === "fileName" ? "title" : "text"}
        </label>
      </div>

      <div class="map">
        <span class="label">Value map {rule.map.length === 0 ? "(none: values are used as found)" : ""}</span>
        {#each rule.map as pair, j (j)}
          <div class="pair">
            <input bind:value={pair[0]} oninput={changed} aria-label="From" placeholder="jr" />
            <span class="muted">→</span>
            <input bind:value={pair[1]} oninput={changed} aria-label="To" placeholder="journal" />
            <button
              onclick={() => {
                rule.map.splice(j, 1);
                changed();
              }}
              aria-label="Remove mapping">×</button
            >
          </div>
        {/each}
        <div>
          <button
            onclick={() => {
              rule.map.push(["", ""]);
              changed();
            }}>Add mapping</button
          >
          <button onclick={() => test(i)}>Test on sample</button>
        </div>
      </div>
      {#if testedIndex === i}{@render testResult()}{/if}
    </section>
  {/each}

  <div class="actions sticky">
    <button onclick={add} disabled={!loaded}>Add rule</button>
    <button onclick={resetDefaults} disabled={!loaded}>Reset to defaults</button>
    <span class="spacer"></span>
    {#if error}<span class="error" role="alert">{error}</span>{/if}
    {#if status}<span class="muted">{status}</span>{/if}
    <button class="primary" onclick={save} disabled={!dirty || saving}>{saving ? "Saving…" : "Save rules"}</button>
  </div>
</div>

<style>
  .rules {
    display: grid;
    gap: 1rem;
  }

  p {
    margin: 0;
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  section {
    display: grid;
    gap: 0.6rem;
    padding: 0.9rem;
    border: 1px solid var(--border);
    border-radius: 8px;
  }

  .rule.disabled {
    opacity: 0.65;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .head input[type="checkbox"] {
    width: auto;
  }

  .name {
    flex: 1;
    font-weight: 600;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 0.6rem;
    align-items: end;
  }

  .wide {
    grid-column: 1 / -1;
  }

  .mono,
  textarea,
  pre,
  code {
    font-family: var(--mono);
  }

  .check {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 0.4rem;
    color: inherit;
  }

  .check input {
    width: auto;
  }

  .map {
    display: grid;
    gap: 0.35rem;
  }

  .label {
    font-size: 0.9em;
    color: var(--muted);
  }

  .pair {
    display: grid;
    grid-template-columns: 1fr auto 1fr auto;
    align-items: center;
    gap: 0.35rem;
    max-width: 420px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }

  .spacer {
    flex: 1;
  }

  .sticky {
    position: sticky;
    bottom: 0;
    padding: 0.6rem 0;
    background: var(--surface);
    border-top: 1px solid var(--border);
  }

  .result {
    display: grid;
    gap: 0.4rem;
    padding: 0.6rem;
    border-radius: 6px;
    background: var(--bg);
    font-size: 0.9em;
  }

  dl {
    margin: 0;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.2rem 0.75rem;
  }

  dt {
    color: var(--muted);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  pre {
    margin: 0.4rem 0 0;
    max-height: 30vh;
    overflow: auto;
    white-space: pre-wrap;
    font-size: 0.9em;
  }
</style>
