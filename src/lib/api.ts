// Typed wrappers around the Rust commands. The backend owns all vault access.
import { invoke } from "@tauri-apps/api/core";
import type { FolderSize } from "$lib/folderTree";

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
  readStatus: ReadStatus;
  createdAt: number;
  updatedAt: number;
  /** UTF-8 bytes of the body. Set on the explorer tree's pages and on a saved note's
   * summary; search hits and other lists leave it out. */
  size?: number;
};

/** Mirrors `db::ReadStatus`: marked by hand, stored in the vault (not the frontmatter). */
export type ReadStatus = "unread" | "partlyRead" | "read";
/** Mirrors `db::ReadFilter`; `notFinished` is unread or partly read. */
export type ReadFilter = ReadStatus | "notFinished";

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
/** The graph's scope; `folder` null is the open note's folder. */
export type StoredGraphScope = { kind: "folder" | "tag" | "search" | "all"; folder: string | null; tag: string };
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
/** What a sidebar search covers: the selected folder or the whole vault. */
export type SearchScope = "folder" | "vault";
/** How a sidebar search finds notes: by their words, or by meaning and words fused (#146). */
export type SearchMode = "words" | "meaning";

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
  /** Left out: words. */
  mode?: SearchMode;
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
  /** null for any read status. */
  readStatus: ReadFilter | null;
  limit: number;
  offset: number;
};

/** Which side of a meaning search found a note: its words, its vector (with the cosine), or both. */
export type SearchWhy = { words: boolean; cosine: number | null };
/** `snippet` marks matches with the control characters below; a hit found by meaning
 * alone has the start of its body. `why` is set in meaning mode only. */
export type SearchHit = { note: NoteSummary; snippet: string | null; why?: SearchWhy };
/** `newestFirst`: too many notes matched to rank them, so these are the newest matches.
 * `byMeaning`: a meaning search used the note vectors; `top`: its list was cut at this
 * many notes; `wordsOnly`: why a meaning search found notes by their words only.
 * `cancelled`: a newer search replaced this one; drop the result. */
export type SearchResult = {
  hits: SearchHit[];
  newestFirst: boolean;
  byMeaning: boolean;
  top: number | null;
  wordsOnly: string | null;
  cancelled: boolean;
};

/** Emitted while opening a vault runs a schema upgrade that rebuilds derived data. */
export const VAULT_UPGRADE_EVENT = "vault-upgrade";
/** `step`: which upgrade is running, so the unlock screen can say what it does. */
export type VaultUpgrade = { step: "searchIndex" | "folderSizes"; done: number; total: number };
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
  /** Each folder's own byte total (not its subfolders'), apart from `folders` so a save
   * only changes numbers, not the tree. */
  folderSizes: () => invoke<FolderSize[]>("folder_sizes"),
  /** Notes directly in `folder` ("" for the root), sorted by title. */
  folderNotes: (folder: string, offset: number, limit: number) =>
    invoke<NoteSummary[]>("folder_notes", { folder, offset, limit }),
  sidebarView: () => invoke<SidebarView>("sidebar_view"),
  /** The folder selected in the sidebar, stored in the vault; null when none. */
  sidebarFolder: () => invoke<string | null>("sidebar_folder"),
  setSidebarFolder: (folder: string | null) => invoke<void>("set_sidebar_folder", { folder }),
  /** The note open when the app was last left, if it is still live; not recorded as opened. */
  lastOpenedNote: () => invoke<OpenedNote | null>("last_opened_note"),
  graphScope: () => invoke<StoredGraphScope | null>("graph_scope"),
  setGraphScope: (scope: StoredGraphScope) => invoke<void>("set_graph_scope", { scope }),
  setSidebarView: (view: SidebarView) => invoke<void>("set_sidebar_view", { view }),
  sidebarSearchScope: () => invoke<SearchScope>("sidebar_search_scope"),
  setSidebarSearchScope: (scope: SearchScope) => invoke<void>("set_sidebar_search_scope", { scope }),
  sidebarSearchMode: () => invoke<SearchMode>("sidebar_search_mode"),
  setSidebarSearchMode: (mode: SearchMode) => invoke<void>("set_sidebar_search_mode", { mode }),
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
  /** Marks notes; resolves to the ids whose status changed. Leaves body and dates alone. */
  setReadStatus: (ids: number[], status: ReadStatus) => invoke<number[]>("set_read_status", { ids, status }),
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
  /** Whether a meaning search can embed its query at once (else it loads the model first). */
  meaningModelLoaded: () => invoke<boolean>("meaning_model_loaded"),
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
  /** Which GPUs there are, whether models made for the GPU may use one, and where the model runs. */
  gpuStatus: () => invoke<GpuStatus>("gpu_status"),
  /** Lets models made for the GPU use it (this computer only); applies from the next indexing pass. */
  setIndexGpu: (on: boolean) => invoke<void>("set_index_gpu", { on }),
  setIndexAllCores: (on: boolean) => invoke<void>("set_index_all_cores", { on }),
  /** The vault's model as the indexing job last saw it; changes arrive as `MODEL_STATUS_EVENT`. */
  modelStatus: () => invoke<ModelStatus>("model_status"),
  /** Everything Settings → Model shows; checks each model's files. */
  modelOverview: () => invoke<ModelOverview>("model_overview"),
  /** Switches the vault to an available model; every note is re-embedded with it. */
  setEmbeddingModel: (model: string) => invoke<void>("set_embedding_model", { model }),
  /** Checks the model files again and restarts indexing if it had stopped on them. */
  recheckModels: () => invoke<void>("recheck_models"),
  /** How each model's last download this run went; changes arrive as `MODEL_DOWNLOAD_EVENT`. */
  modelDownloads: () => invoke<DownloadStatus[]>("model_downloads"),
  /** Downloads an optional model from its pinned URL. The only thing that makes Hush go online. */
  downloadModel: (model: string) => invoke<void>("download_model", { model }),
  /** Stops a download; the next one continues where it stopped. */
  cancelModelDownload: (model: string) => invoke<void>("cancel_model_download", { model }),
  /** Copies picked files into the models folder if they match the model's checksums; resolves to the names stored. */
  importModelFiles: (model: string, paths: string[]) => invoke<string[]>("import_model_files", { model, paths }),
  /** Deletes a downloaded or imported model; resolves to the bytes freed. */
  removeModel: (model: string) => invoke<number>("remove_model", { model }),
  /** Notes closest in meaning, most similar first, with score and reasons; `null` if the note isn't indexed yet. */
  relatedNotes: (id: number, limit: number) => invoke<RelatedNote[] | null>("related_notes", { id, limit }),

  /** Notes that link to `id`, by title, each with the line around its link. */
  backlinks: (id: number) => invoke<Backlink[]>("backlinks", { id }),
  /** What note `id` links to, by target text. */
  outgoingLinks: (id: number) => invoke<OutgoingLink[]>("outgoing_links", { id }),
  /** What a link target (between `[[` and `]]`, without heading or alias) resolves to. */
  resolveLink: (target: string) => invoke<OutgoingLink>("resolve_link", { target }),
  /** Creates and opens the note an unresolved link target names (`path/Title`). */
  createLinkedNote: (target: string) => invoke<OpenedNote>("create_linked_note", { target }),
  /** The collapsed sections of the details panel, stored in the vault. */
  collapsedSections: () => invoke<DetailsSection[]>("collapsed_sections"),
  setSectionCollapsed: (section: DetailsSection, collapsed: boolean) =>
    invoke<void>("set_section_collapsed", { section, collapsed }),
  /** The subgraph of a scope for the graph view; never the whole vault uncapped. */
  graph: (scope: GraphScope, options: GraphOptions) => invoke<Graph>("graph", { scope, options }),
  /** What the background jobs are doing; changes arrive as `ACTIVITY_EVENT`. */
  activityStatus: () => invoke<ActivitySnapshot>("activity_status"),
  /** The counts in the snapshot are only refreshed while the Activity panel is open. */
  setActivityPanelOpen: (open: boolean) => invoke<void>("set_activity_panel_open", { open }),
  /** What Hush stores and derives for a note. */
  noteInternals: (id: number) => invoke<NoteInternals>("note_internals", { id }),
};

/** The background jobs (see `activity::Job`), in the order the panel lists them. */
export type ActivityJob =
  | "embedding"
  | "related"
  | "training"
  | "safetyNet"
  | "import"
  | "discover"
  | "migration"
  | "retag";
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

/** Mirrors `graph::GraphScope`. */
export type GraphScope = { kind: "local"; id: number; depth: number } | { kind: "global"; scope: GraphScopeNotes };
/** Mirrors `db::GraphScopeNotes`; a tag takes its nested tags too. */
export type GraphScopeNotes =
  | { kind: "all" }
  | { kind: "folder"; folder: string }
  | { kind: "tag"; tag: string }
  | { kind: "search"; query: SearchQuery };
/** Mirrors `graph::GraphOptions`. */
export type GraphOptions = {
  meaning: boolean;
  tags: boolean;
  /** With `tags`, also tags only one note in the vault has. */
  rareTags: boolean;
  ghosts: boolean;
  /** Lowest cosine similarity of a meaning edge. */
  threshold: number;
  /** Meaning edges per note, at most. */
  neighbours: number;
  /** Most notes in the graph. */
  cap: number;
};
export type GraphNode = {
  /** `n<id>`, `t:<tag>` or `g:<target>`. */
  key: string;
  kind: "note" | "tag" | "ghost";
  id: number | null;
  label: string;
  folder: string;
  topic: string | null;
  tags: string[];
  /** Links from the local graph's centre. */
  depth: number | null;
};
export type GraphEdge = { source: string; target: string; kind: "link" | "meaning" | "tag"; score: number | null };
export type Graph = {
  nodes: GraphNode[];
  edges: GraphEdge[];
  /** Notes in the scope (for a local graph, those found before the cap stopped it). */
  total: number;
  capped: boolean;
  meaningUnavailable: boolean;
};

/** Mirrors `db::Resolution`: what a link target means. */
export type Resolution = { kind: "unique"; ids: number } | { kind: "ambiguous"; ids: number[] } | { kind: "missing" };
export type Backlink = { note: NoteSummary; ambiguous: boolean; snippet: string };
/** `notes` are the notes the target resolves to (none when missing). */
export type OutgoingLink = { target: string; resolution: Resolution; notes: NoteSummary[] };
/** Mirrors `db::Reasons`: why a related note relates to the open one, besides meaning. */
export type Reasons = {
  linksTo: boolean;
  linkedFrom: boolean;
  bothLinkTo: TitledNote[];
  bothLinkedFrom: TitledNote[];
  via: TitledNote[];
  sharedTags: string[];
  sameFolder: boolean;
  sameAuthor: string | null;
  sameTopic: string | null;
};
/** `score` is the exact cosine similarity, -1 to 1. */
export type RelatedNote = { note: NoteSummary; score: number; reasons: Reasons };
export type DetailsSection = "related" | "backlinks" | "outgoing";

/** The embedding job's state (see `IndexStatus` in embed/job.rs). */
export type IndexStatus =
  | { state: "idle" }
  /** `done` of `total` notes have a vector, `complete` of them from all of the note
   * rather than just its first chunk. */
  | { state: "indexing"; done: number; complete: number; total: number; etaSecs: number | null }
  /** An import is running; indexing carries on when it ends. */
  | { state: "paused" }
  | { state: "failed"; message: string };

export const INDEX_STATUS_EVENT = "index-status";

/** How loading a model went this session (see `LoadState` in embed/models.rs). */
export type LoadState =
  | { state: "notLoaded" }
  | { state: "loading" }
  | { state: "ready" }
  | { state: "missing"; file: string }
  | { state: "checksum"; file: string }
  | { state: "unknown" }
  | { state: "failed"; message: string };
export type ReducedReason = "missing" | "checksum" | "unknown" | "failed" | "loading" | "firstIndex";
/** Mirrors `embed::models::ModelStatus`. `reduced` is set while features that need a model are off. */
export type ModelStatus = {
  model: string | null;
  readyModel: string | null;
  load: LoadState;
  reduced: { reason: ReducedReason; message: string } | null;
};
export type Availability =
  /** `downloaded`: in the app data folder (downloaded or imported), so it can be removed. */
  | { state: "available"; bytes: number; downloaded: boolean }
  | { state: "missing"; file: string }
  | { state: "checksum"; file: string }
  | { state: "unreadable"; file: string; message: string };
export type ModelInfo = {
  id: string;
  name: string;
  licence: string;
  licenceUrl: string;
  dims: number;
  maxTokens: number;
  /** Ships with Hush; the others can be downloaded or imported. */
  bundled: boolean;
  /** Bytes to download. */
  size: number;
  /** Where a download comes from. */
  host: string;
  availability: Availability;
  /** Notes per second last measured on this machine. */
  rate: number | null;
  /** Made for the GPU: runs through DirectML when this computer has a usable one. */
  gpu: boolean;
};
export type GpuAdapter = { name: string; dedicatedBytes: number; discrete: boolean };
/** Settings → Indexing's GPU line (see `GpuStatus` in commands/embeddings.rs). */
export type GpuStatus = {
  adapters: GpuAdapter[];
  /** The separate GPU a GPU model would use, if any. */
  best: GpuAdapter | null;
  /** Settings allows the GPU. */
  enabled: boolean;
  /** The model last loaded, where it runs, and why not on the GPU when it could have. */
  model: string | null;
  device: "cpu" | "gpu" | null;
  reason: string | null;
  /** The name of the model made for the GPU, if Hush knows one. */
  gpuModel: string | null;
};
export const GPU_STATUS_EVENT = "gpu-status";
export type ModelOverview = {
  status: ModelStatus;
  /** The vault's model, from its settings. */
  model: string;
  notes: number;
  models: ModelInfo[];
  defaultRate: number;
};
export const MODEL_STATUS_EVENT = "model-status";
export type DownloadStatus = { model: string } & (
  | { state: "running"; done: number; total: number; bytesPerSec: number | null }
  | { state: "done" }
  | { state: "cancelled"; done: number; total: number }
  | { state: "failed"; message: string }
);
export const MODEL_DOWNLOAD_EVENT = "model-download";
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
