// Typed wrappers around the Rust commands. The backend owns all vault access.
import { invoke } from "@tauri-apps/api/core";

export type VaultInfo = { path: string; name: string };
export type RecentVault = { path: string; name: string; exists: boolean };
export type LockReason = "manual" | "idle";

export type NoteSummary = {
  id: number;
  folder: string;
  title: string;
  author: string | null;
  topic: string | null;
  createdAt: number;
  updatedAt: number;
};

export type Note = NoteSummary & { body: string; tags: string[] };

/** Metadata parsed from the body; the frontmatter is its single source of truth. */
export type NoteMeta = {
  author: string | null;
  topic: string | null;
  tags: string[];
  frontmatterTags: string[];
  attributes: [string, string][];
};

export type OpenedNote = { note: Note; meta: NoteMeta };
export type SavedNote = { summary: NoteSummary; meta: NoteMeta };

export type DateField = "created" | "updated";

/** Mirrors `db::SearchQuery`. Dates are Unix millis; `from` is inclusive, `to` exclusive. */
export type SearchQuery = {
  text: string;
  tags: string[];
  topic: string | null;
  author: string | null;
  folder: string | null;
  dateField: DateField;
  from: number | null;
  to: number | null;
  attributes: [string, string][];
  limit: number;
  offset: number;
};

/** `snippet` marks matches with the control characters below. */
export type SearchHit = { note: NoteSummary; snippet: string | null };
export const HIGHLIGHT_START = "\u0002";
export const HIGHLIGHT_END = "\u0003";

export type Facets = { tags: string[]; topics: string[]; authors: string[]; attributeKeys: string[] };

/** The shape of every command error (see `CommandError` in commands/mod.rs). */
export type ApiError = { kind: string; message: string };

export const LOCKED_EVENT = "vault-locked";
export const CLOSE_EVENT = "close-requested";

export const api = {
  recentVaults: () => invoke<RecentVault[]>("recent_vaults"),
  forgetRecentVault: (path: string) => invoke<void>("forget_recent_vault", { path }),
  createVault: (path: string, passphrase: string) => invoke<VaultInfo>("create_vault", { path, passphrase }),
  openVault: (path: string, passphrase: string) => invoke<VaultInfo>("open_vault", { path, passphrase }),
  lockVault: () => invoke<void>("lock_vault"),
  vaultStatus: () => invoke<VaultInfo | null>("vault_status"),
  touch: () => invoke<void>("touch"),
  exitApp: () => invoke<void>("exit_app"),
  autoLockMinutes: () => invoke<number>("auto_lock_minutes"),
  setAutoLockMinutes: (minutes: number) => invoke<void>("set_auto_lock_minutes", { minutes }),
  changePassphrase: (current: string, next: string) => invoke<void>("change_passphrase", { current, new: next }),

  folders: () => invoke<string[]>("folders"),
  createNote: (folder: string) => invoke<OpenedNote>("create_note", { folder }),
  openNote: (id: number) => invoke<OpenedNote>("open_note", { id }),
  saveNote: (id: number, body: string) => invoke<SavedNote>("save_note", { id, body }),
  renameNote: (id: number, folder: string, title: string) => invoke<OpenedNote>("rename_note", { id, folder, title }),
  deleteNote: (id: number) => invoke<void>("delete_note", { id }),
  editFrontmatter: (body: string, key: string, values: string[]) =>
    invoke<string>("edit_frontmatter", { body, key, values }),
  defaultAuthor: () => invoke<string | null>("default_author"),
  setDefaultAuthor: (author: string) => invoke<void>("set_default_author", { author }),

  searchNotes: (query: SearchQuery) => invoke<SearchHit[]>("search_notes", { query }),
  searchFacets: () => invoke<Facets>("search_facets"),
  attributeValues: (key: string) => invoke<string[]>("attribute_values", { key }),
};

export function isApiError(e: unknown): e is ApiError {
  return typeof e === "object" && e !== null && "kind" in e && "message" in e;
}

export function errorMessage(e: unknown): string {
  if (isApiError(e)) {
    const message = e.message;
    return message.charAt(0).toUpperCase() + message.slice(1) + ".";
  }
  return String(e);
}
