<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import { normalizeFolder } from "$lib/markdown";

  type Props = {
    folders: string[];
    /** `null` is "All notes". */
    selected: string | null;
    onselect: (folder: string | null) => void;
    oncreate: (folder: string) => void;
  };
  let { folders, selected, onselect, oncreate }: Props = $props();

  type Node = { name: string; path: string; children: Node[] };

  const tree = $derived.by(() => {
    const root: Node = { name: "", path: "", children: [] };
    for (const folder of folders) {
      let node = root;
      for (const part of folder.split("/").filter(Boolean)) {
        let child = node.children.find((c) => c.name.toLowerCase() === part.toLowerCase());
        if (!child) {
          child = { name: part, path: node.path ? `${node.path}/${part}` : part, children: [] };
          node.children.push(child);
        }
        node = child;
      }
    }
    const sort = (nodes: Node[]) => {
      nodes.sort((a, b) => a.name.localeCompare(b.name));
      nodes.forEach((n) => sort(n.children));
    };
    sort(root.children);
    return root.children;
  });

  const expanded = new SvelteSet<string>();
  let naming = $state(false);
  let newName = $state("");

  function select(path: string | null) {
    onselect(path);
    // Keep the selected folder's ancestors open.
    const parts = (path ?? "").split("/");
    for (let i = 1; i < parts.length; i++) expanded.add(parts.slice(0, i).join("/"));
  }

  function toggle(path: string) {
    if (expanded.has(path)) expanded.delete(path);
    else expanded.add(path);
  }

  function create(e: SubmitEvent) {
    e.preventDefault();
    const name = normalizeFolder(newName);
    naming = false;
    newName = "";
    if (!name) return;
    const path = selected ? `${selected}/${name}` : name;
    if (selected) expanded.add(selected);
    oncreate(path);
    select(path);
  }

  function focusOnMount(el: HTMLInputElement) {
    el.focus();
  }
</script>

<nav aria-label="Folders">
  <div class="heading">
    <span>Folders</span>
    <button class="small" onclick={() => (naming = true)} title="New folder inside the selected folder">+ Folder</button>
  </div>

  {#if naming}
    <form onsubmit={create}>
      <input
        placeholder={selected ? `New folder in ${selected}` : "New folder"}
        aria-label="New folder name"
        bind:value={newName}
        onblur={() => (naming = false)}
        onkeydown={(e) => e.key === "Escape" && (naming = false)}
        {@attach focusOnMount}
      />
    </form>
  {/if}

  <ul>
    <li>
      <div class="row" class:selected={selected === null}>
        <span class="twisty"></span>
        <button class="name" onclick={() => select(null)}>All notes</button>
      </div>
    </li>
    {@render branch(tree, 0)}
  </ul>
</nav>

{#snippet branch(nodes: Node[], depth: number)}
  {#each nodes as node (node.path)}
    <li>
      <div class="row" class:selected={selected?.toLowerCase() === node.path.toLowerCase()} style:padding-left="{depth * 14}px">
        {#if node.children.length > 0}
          <button
            class="twisty"
            aria-label="{expanded.has(node.path) ? 'Collapse' : 'Expand'} {node.name}"
            aria-expanded={expanded.has(node.path)}
            onclick={() => toggle(node.path)}
          >
            {expanded.has(node.path) ? "▾" : "▸"}
          </button>
        {:else}
          <span class="twisty"></span>
        {/if}
        <button class="name" title={node.path} onclick={() => select(node.path)}>{node.name}</button>
      </div>
      {#if node.children.length > 0 && expanded.has(node.path)}
        <ul>{@render branch(node.children, depth + 1)}</ul>
      {/if}
    </li>
  {/each}
{/snippet}

<style>
  nav {
    display: grid;
    gap: 0.25rem;
  }

  .heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 0.8em;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }

  .small {
    padding: 0.1em 0.5em;
    font-size: 0.95em;
    text-transform: none;
    letter-spacing: normal;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .row {
    display: flex;
    align-items: center;
    border-radius: 4px;
  }

  .row:hover {
    background: var(--active-line);
  }

  .row.selected {
    background: var(--selection);
  }

  .row button {
    border: none;
    background: none;
    padding: 0.15em 0.25em;
  }

  .twisty {
    width: 1.4em;
    flex: none;
    color: var(--muted);
    text-align: center;
  }

  .name {
    flex: 1;
    min-width: 0;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
