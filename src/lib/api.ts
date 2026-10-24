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

/** The left pane: the explorer tree, or the folder tree with a flat list by updated. */
export type SidebarView = "tree" | "recent";

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

export type RuleSource = "fileName" | "frontmatter" | "firstLines" | "body";

/** `tag`, `author`, `author_email`, `topic`, `created_at` or `attribute:<key>`. */
export type RuleTarget = string;

/** Mirrors `import::rules::Rule`. */
export type Rule = {
  name: string;
  enabled: boolean;
  source: RuleSource;
  lines: number;
  pattern: string;
  target: RuleTarget;
  map: [string, string][];
  strip: boolean;
};

export type RuleOutput = {
  tags: string[];
  author: string | null;
  authorEmail: string | null;
  topic: string | null;
  createdAt: number | null;
  attributes: [string, string[]][];
  fileName: string;
  content: string;
  warnings: string[];
};

export type FileMessage = { path: string; message: string };

export type ImportReport = {
  dryRun: boolean;
  files: number;
  imported: number;
  skipped: FileMessage[];
  warnings: FileMessage[];
  warningCount: number;
  ambiguousLinks: { note: string; target: string }[];
  samples: { path: string; before: string; after: string }[];
  /** Files whose format can't be read yet, most common first. */
  unsupported: { format: string; count: number }[];
  cancelled: boolean;
};

export type ImportProgress = { done: number; total: number };

export type ImportFinished = { status: "done"; report: ImportReport } | { status: "failed"; error: ApiError };

export const IMPORT_PROGRESS_EVENT = "import-progress";
export const IMPORT_FINISHED_EVENT = "import-finished";

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
  /** Notes directly in `folder` ("" for the root), sorted by title. */
  folderNotes: (folder: string, offset: number, limit: number) =>
    invoke<NoteSummary[]>("folder_notes", { folder, offset, limit }),
  sidebarView: () => invoke<SidebarView>("sidebar_view"),
  setSidebarView: (view: SidebarView) => invoke<void>("set_sidebar_view", { view }),
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

  /** Returns at once; progress and the report arrive as events. */
  startImport: (source: string, dryRun: boolean) => invoke<void>("start_import", { source, dryRun }),
  cancelImport: () => invoke<void>("cancel_import"),
  importRules: () => invoke<Rule[]>("import_rules"),
  setImportRules: (rules: Rule[]) => invoke<void>("set_import_rules", { rules }),
  defaultImportRules: () => invoke<Rule[]>("default_import_rules"),
  testImportRules: (rules: Rule[], fileName: string, sample: string) =>
    invoke<RuleOutput>("test_import_rules", { rules, fileName, sample }),
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
