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
  /** 1–5 stars, or null when unrated. */
  rating: number | null;
  createdAt: number;
  updatedAt: number;
};

export type Note = NoteSummary & { body: string; tags: string[] };

/** Metadata parsed from the body; the frontmatter is its single source of truth. */
export type NoteMeta = {
  author: string | null;
  topic: string | null;
  /** A valid `rating` (1–5); any other value is listed under `attributes`. */
  rating: number | null;
  tags: string[];
  frontmatterTags: string[];
  attributes: [string, string][];
};

export type OpenedNote = { note: Note; meta: NoteMeta };
/** A note in the trash; `folder` is where it was deleted from. */
export type TrashedNote = { id: number; folder: string; title: string; deletedAt: number };
/** One row of the trash: a folder deleted as a whole (its notes share `deletedAt`), or a note. */
export type TrashEntry =
  | { kind: "folder"; deletedAt: number; folder: string; notes: number }
  | ({ kind: "note" } & TrashedNote);
/** What deleting a folder would move to the trash. */
export type FolderContents = { notes: number; subfolders: number };
export type SavedNote = { summary: NoteSummary; meta: NoteMeta };

/** Mirrors `db::MetadataEdit`. An empty topic or author, or a null rating, removes the key. */
export type MetadataEdit =
  | { kind: "addTag"; value: string }
  | { kind: "removeTag"; value: string }
  | { kind: "setTopic"; value: string }
  | { kind: "setAuthor"; value: string }
  | { kind: "setRating"; value: number | null };
export type TitledNote = { id: number; folder: string; title: string };
export type MetadataEditReport = {
  /** Notes whose body was rewritten. */
  changed: number[];
  /** Notes the edit left as they were. */
  unchanged: number;
  /** After removing a tag: notes that still have it inline. */
  stillInline: TitledNote[];
  /** Skipped: their frontmatter can't be read. */
  unreadable: TitledNote[];
};

/** A tag used on notes, described, or both (see `db::TagInfo`). */
export type TagInfo = { tag: string; notes: number; description: string };
/** How a tag was scored (see `db::ScoreMethod`). */
export type ScoreMethod = "classifier" | "description" | "descriptionAndExamples";
/**
 * A tag suggested for a note: `score` is the classifier's probability, or the similarity
 * to its description (blended with its examples).
 */
export type TagSuggestion = { tag: string; score: number; method: ScoreMethod };
/** A tag's classifier (see `db::TagTraining`). */
export type TagTraining = {
  tag: string;
  positives: number;
  negatives: number;
  rejected: number;
  status: "trained" | "waiting" | "descriptionOnly" | "needsExamples";
  /** More positive examples needed before it is trained. */
  needed: number;
  precision: number | null;
  recall: number | null;
  trainedAt: number | null;
};
/** The threshold of the model suggestions use; `threshold` is null for an unknown model without one. */
export type SuggestionThreshold = { model: string; threshold: number | null; default: number | null };
/** A note in a discovered group; `score` is its similarity to the group's centre. */
export type GroupNote = { id: number; folder: string; title: string; score: number };
/** Notes grouped by meaning (see `discover::Group`): `size` notes, of which `notes`
 * are the ones nearest the centre, nearest first. */
export type DiscoverGroup = { words: string[]; size: number; notes: GroupNote[] };
/** `model` is `null` while no model is ready; `k` is the number of groups asked for;
 * `sampled` is how many notes the groups were computed from (the rest joined the
 * nearest group). `run` names the groups for "select all". */
export type Discovery = {
  run: number;
  model: string | null;
  notes: number;
  sampled: number;
  k: number;
  groups: DiscoverGroup[];
};
/** Which notes Discover tags groups. */
export type DiscoverScope =
  | { kind: "all" }
  | { kind: "untagged" }
  | { kind: "folder"; folder: string }
  | { kind: "search"; query: SearchQuery };

export type DateField = "created" | "updated";

/**
 * The left pane: the explorer tree, the folder tree with a flat list by updated, or one
 * tag's suggestions across the vault.
 */
export type SidebarView = "tree" | "recent" | "suggestions";

/** Mirrors `db::ReviewOrder`. */
export type ReviewOrder = "mostLikely" | "needsReview";
/** A note suggested for the reviewed tag; `excerpt` is the body start without frontmatter. */
export type ReviewNote = { id: number; folder: string; title: string; excerpt: string; score: number };
/** One tag's suggestions across the vault (see `db::SuggestionReview`); `method` is null when it can't be scored. */
export type SuggestionReview = {
  tag: string;
  method: ScoreMethod | null;
  /** The lowest score listed. */
  minScore: number;
  order: ReviewOrder;
  /** How many notes are suggested; `notes` holds the first of them. */
  total: number;
  notes: ReviewNote[];
};

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
  /** Notes rated this many stars or more; null for any. */
  minRating: number | null;
  limit: number;
  offset: number;
};

/** `snippet` marks matches with the control characters below. */
export type SearchHit = { note: NoteSummary; snippet: string | null };
/** `newestFirst`: too many notes matched to rank them, so these are the newest matches.
 * `cancelled`: a newer search replaced this one; drop the result. */
export type SearchResult = { hits: SearchHit[]; newestFirst: boolean; cancelled: boolean };

/** Emitted while opening a vault runs a schema upgrade that rebuilds derived data. */
export const VAULT_UPGRADE_EVENT = "vault-upgrade";
export type VaultUpgrade = { done: number; total: number };
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
  /** The vault folder notes go under, normalised; "" for the root. */
  target: string;
  /** Notes already under the target before the run (0 for the root). */
  existingInTarget: number;
  files: number;
  imported: number;
  /** At most 2,000, files that couldn't be read first; `skippedCount` has the total. */
  skipped: FileMessage[];
  skippedCount: number;
  /** How many skipped files already exist in the vault with that folder and title. */
  alreadyExist: number;
  /** Folders notes go to, with how many, sorted: at most 2,000, the largest. */
  folders: { folder: string; count: number }[];
  folderCount: number;
  warnings: FileMessage[];
  warningCount: number;
  ambiguousLinks: { note: string; target: string }[];
  samples: { path: string; folder: string; before: string; after: string }[];
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
  /** Whether moving notes to the trash asks first (on unless turned off in Settings). */
  confirmTrash: () => invoke<boolean>("confirm_trash"),
  setConfirmTrash: (confirm: boolean) => invoke<void>("set_confirm_trash", { confirm }),
  /** The recently opened notes, most recent first, without trashed ones. */
  recentOpenedNotes: () => invoke<NoteSummary[]>("recent_opened_notes"),
  /** How many recently opened notes are kept, 0–1000; 0 turns the list off. */
  recentOpenedCount: () => invoke<number>("recent_opened_count"),
  setRecentOpenedCount: (count: number) => invoke<void>("set_recent_opened_count", { count }),
  clearRecentOpened: () => invoke<void>("clear_recent_opened"),
  recentOpenedCollapsed: () => invoke<boolean>("recent_opened_collapsed"),
  setRecentOpenedCollapsed: (collapsed: boolean) => invoke<void>("set_recent_opened_collapsed", { collapsed }),
  createNote: (folder: string) => invoke<OpenedNote>("create_note", { folder }),
  /** Opens a note for the user: it moves to the front of the recently opened list. */
  openNote: (id: number) => invoke<OpenedNote>("open_note", { id }),
  /** Reads the open note again (after a move, an edit elsewhere, leaving Settings); not recorded as opened. */
  reloadNote: (id: number) => invoke<OpenedNote>("open_note", { id, record: false }),
  saveNote: (id: number, body: string) => invoke<SavedNote>("save_note", { id, body }),
  renameNote: (id: number, folder: string, title: string) => invoke<OpenedNote>("rename_note", { id, folder, title }),
  /** The default delete: moves the note to the trash. */
  trashNote: (id: number) => invoke<void>("trash_note", { id }),
  /** Bulk delete: moves the notes to the trash, all or nothing. */
  trashNotes: (ids: number[]) => invoke<void>("trash_notes", { ids }),
  /** Moves notes into `folder`, all or nothing; fails with kind `moveClash` on a name clash. */
  moveNotes: (ids: number[], folder: string) => invoke<void>("move_notes", { ids, folder }),
  /**
   * Moves folder `from`, with everything in it, into folder `to` (`""` is the top level),
   * all or nothing; resolves to its new path. Fails with kind `moveClash` on a name clash.
   */
  moveFolder: (from: string, to: string) => invoke<string>("move_folder", { from, to }),
  /** Adds or removes a tag, or sets the topic or author, on several notes, all or nothing. */
  editNotesMetadata: (ids: number[], edit: MetadataEdit) =>
    invoke<MetadataEditReport>("edit_notes_metadata", { ids, edit }),
  /** Deletes a note for good, live or trashed. */
  deleteNote: (id: number) => invoke<void>("delete_note", { id }),
  /** One page of the trash, newest first: folder groups as one entry each, and loose notes. */
  trashEntries: (offset: number, limit: number) => invoke<TrashEntry[]>("trash_entries", { offset, limit }),
  /** How many notes are in the trash, folder groups included. */
  trashCount: () => invoke<number>("trash_count"),
  /** The live notes and subfolders in `folder` and below it. */
  folderContents: (folder: string) => invoke<FolderContents>("folder_contents", { folder }),
  /** Moves `folder` with everything in it to the trash as one entry; resolves to the number of notes. */
  trashFolder: (folder: string) => invoke<number>("trash_folder", { folder }),
  /** Restores a folder group; resolves to the number of notes restored. */
  restoreFolder: (deletedAt: number) => invoke<number>("restore_folder", { deletedAt }),
  /** Deletes a folder group from the trash for good. */
  deleteTrashFolder: (deletedAt: number) => invoke<number>("delete_trash_folder", { deletedAt }),
  restoreNote: (id: number) => invoke<OpenedNote>("restore_note", { id }),
  emptyTrash: () => invoke<number>("empty_trash"),
  trashDays: () => invoke<number>("trash_days"),
  setTrashDays: (days: number) => invoke<void>("set_trash_days", { days }),
  editFrontmatter: (body: string, key: string, values: string[]) =>
    invoke<string>("edit_frontmatter", { body, key, values }),
  defaultAuthor: () => invoke<string | null>("default_author"),
  setDefaultAuthor: (author: string) => invoke<void>("set_default_author", { author }),

  searchNotes: (query: SearchQuery) => invoke<SearchResult>("search_notes", { query }),
  searchFacets: () => invoke<Facets>("search_facets"),
  attributeValues: (key: string) => invoke<string[]>("attribute_values", { key }),

  /** Returns at once; progress and the report arrive as events. */
  startImport: (source: string, target: string, dryRun: boolean) =>
    invoke<void>("start_import", { source, target, dryRun }),
  suggestImportTarget: (source: string) => invoke<string>("suggest_import_target", { source }),
  importTargetNotes: (target: string) => invoke<number>("import_target_notes", { target }),
  cancelImport: () => invoke<void>("cancel_import"),
  importRules: () => invoke<Rule[]>("import_rules"),
  setImportRules: (rules: Rule[]) => invoke<void>("set_import_rules", { rules }),
  defaultImportRules: () => invoke<Rule[]>("default_import_rules"),
  testImportRules: (rules: Rule[], fileName: string, sample: string) =>
    invoke<RuleOutput>("test_import_rules", { rules, fileName, sample }),

  /** Every tag used on a note or described, sorted by name. */
  tagList: () => invoke<TagInfo[]>("tag_list"),
  /** An empty description removes it, and the tag is no longer suggested. */
  setTagDescription: (tag: string, description: string) =>
    invoke<void>("set_tag_description", { tag, description }),
  tagSuggestions: (id: number) => invoke<TagSuggestion[]>("tag_suggestions", { id }),
  /** Adds the tag to the note's frontmatter; resolves to the note fresh. Flush the editor first. */
  acceptTagSuggestion: (id: number, tag: string) => invoke<OpenedNote>("accept_tag_suggestion", { id, tag }),
  rejectTagSuggestion: (id: number, tag: string) => invoke<void>("reject_tag_suggestion", { id, tag }),
  suggestionThreshold: () => invoke<SuggestionThreshold>("suggestion_threshold"),
  /** `null` goes back to the model's default. */
  setSuggestionThreshold: (threshold: number | null) => invoke<void>("set_suggestion_threshold", { threshold }),
  /** The first `limit` notes suggested for `tag` across the vault, in `order`. */
  reviewTagSuggestions: (tag: string, order: ReviewOrder, limit: number) =>
    invoke<SuggestionReview>("review_tag_suggestions", { tag, order, limit }),
  /** Adds the tag to the notes and records them as accepted, all or nothing. Flush the editor first. */
  acceptTagSuggestions: (tag: string, ids: number[]) =>
    invoke<MetadataEditReport>("accept_tag_suggestions", { tag, ids }),
  /** Records the tag as rejected on the notes, all or nothing; the notes don't change. */
  rejectTagSuggestions: (tag: string, ids: number[]) => invoke<void>("reject_tag_suggestions", { tag, ids }),
  /** A training pass now; resolves to whether a classifier changed (then `TAGS_TRAINED_EVENT` fires too). */
  retrainTags: () => invoke<boolean>("retrain_tags"),
  /** Per-tag classifier state; `TAGS_TRAINED_EVENT` says when it changed. */
  tagTraining: () => invoke<TagTraining[]>("tag_training"),

  /** Groups notes by meaning; `groups: null` picks the default. Resolves to `null` when cancelled. */
  discoverTags: (groups: number | null, scope: DiscoverScope) =>
    invoke<Discovery | null>("discover_tags", { options: { groups, scope } }),
  cancelDiscoverTags: () => invoke<void>("cancel_discover_tags"),
  /** Saves the description (empty leaves it alone) and adds the tag to `ids`, or with
   * `wholeGroup` to every member of group `group` of run `run`; all or nothing. */
  createTagFromGroup: (
    tag: string,
    description: string,
    ids: number[],
    wholeGroup: { run: number; group: number } | null = null,
  ) => invoke<MetadataEditReport>("create_tag_from_group", { tag, description, ids, wholeGroup }),

  /** The background indexing job; changes arrive as `INDEX_STATUS_EVENT`. */
  indexStatus: () => invoke<IndexStatus>("index_status"),
  /** Whether indexing uses every core on this machine (else half). */
  indexAllCores: () => invoke<boolean>("index_all_cores"),
  setIndexAllCores: (on: boolean) => invoke<void>("set_index_all_cores", { on }),
  /** Notes closest in meaning, most similar first; `null` if the note isn't indexed yet. */
  relatedNotes: (id: number, limit: number) => invoke<NoteSummary[] | null>("related_notes", { id, limit }),

  /** What the background jobs are doing; changes arrive as `ACTIVITY_EVENT`. */
  activityStatus: () => invoke<ActivitySnapshot>("activity_status"),
  /** The counts in the snapshot are only refreshed while the Activity panel is open. */
  setActivityPanelOpen: (open: boolean) => invoke<void>("set_activity_panel_open", { open }),
  /** What Hush stores and derives for a note. */
  noteInternals: (id: number) => invoke<NoteInternals>("note_internals", { id }),
};

/** The background jobs (see `activity::Job`), in the order the panel lists them. */
export type ActivityJob = "embedding" | "related" | "training" | "safetyNet" | "import" | "discover" | "migration";
export type ActivityState = "idle" | "waiting" | "running" | "paused" | "stopped" | "failed";
export type JobStatus = {
  job: ActivityJob;
  state: ActivityState;
  /** What it is doing right now. */
  doing: string | null;
  /** Why it is waiting, paused, failed or idle. */
  reason: string | null;
  done: number | null;
  /** `null` when the end isn't known. */
  total: number | null;
  /** Items per second over the last minute. */
  rate: number | null;
  etaSecs: number | null;
  startedAt: number | null;
  lastRun: { finishedAt: number; durationMs: number; summary: string } | null;
};
export type ActivityLogEntry = { seq: number; at: number; job: ActivityJob; error: boolean; message: string };
/** Counted while the Activity panel is open; `null` until first counted. */
export type ActivityStats = {
  countedAt: number | null;
  notes: number | null;
  withVector: number | null;
  queued: number | null;
  model: string | null;
  readyModel: string | null;
  modelLoaded: boolean;
  coresUsed: number | null;
  coresTotal: number | null;
  relatedNotes: number | null;
  relatedBytes: number | null;
  heads: { tag: string; model: string; trainedAt: number }[];
  schemaVersion: number | null;
};
export type ActivitySnapshot = {
  revision: number;
  jobs: JobStatus[];
  stats: ActivityStats;
  /** Newest first, at most 500. Memory only: never stored. */
  log: ActivityLogEntry[];
};
export type NoteInternals = {
  id: number;
  folder: string;
  createdAt: number;
  updatedAt: number;
  words: number;
  chars: number;
  /** SHA-256 of the text the embedding job embeds; null when there is nothing to embed. */
  contentHash: string | null;
  vectors: { model: string; hash: string; current: boolean }[];
  /** The model the job embeds with now. */
  model: string;
  queued: boolean;
  backlinks: number;
  outgoing: number;
  tags: { tag: string; source: "manual" | "suggestion" }[];
};

/** The embedding job's state (see `IndexStatus` in embed/job.rs). */
export type IndexStatus =
  | { state: "idle" }
  | { state: "indexing"; done: number; total: number; etaSecs: number | null }
  /** An import is running; indexing carries on when it ends. */
  | { state: "paused" }
  | { state: "failed"; message: string };

export const INDEX_STATUS_EVENT = "index-status";
/** Emitted with an `ActivitySnapshot`, at most four times a second. */
export const ACTIVITY_EVENT = "activity-changed";
/** The related-notes index changed (a note was re-embedded, trashed, restored or deleted). */
export const RELATED_CHANGED_EVENT = "related-changed";
/** Emitted after the training job trained or dropped a tag's classifier. */
export const TAGS_TRAINED_EVENT = "tags-trained";

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
