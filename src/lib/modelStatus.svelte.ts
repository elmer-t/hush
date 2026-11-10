// The vault's model as the indexing job last saw it, shared by every place that says
// why a feature is off (see docs/decisions.md, Model section). Module state, so the
// footer, the Activity panel and the details panel agree; lock reloads the page.
import { listen } from "@tauri-apps/api/event";
import { api, INDEX_STATUS_EVENT, MODEL_STATUS_EVENT, type IndexStatus, type ModelStatus } from "$lib/api";

export const model = $state<{
  status: ModelStatus | null;
  index: IndexStatus;
  /** Opens Settings → Model; set by the workspace. */
  open: (() => void) | null;
}>({ status: null, index: { state: "idle" }, open: null });

let watching = false;

/** Starts following the status; once per page. */
export function watchModel() {
  if (watching) return;
  watching = true;
  void listen<ModelStatus>(MODEL_STATUS_EVENT, (e) => (model.status = e.payload));
  void listen<IndexStatus>(INDEX_STATUS_EVENT, (e) => (model.index = e.payload));
  api.modelStatus().then((s) => (model.status = s), () => {});
  api.indexStatus().then((s) => (model.index = s), () => {});
}

/** Why features that need a model are off, as one short line; `null` while they work. */
export function reducedText(): string | null {
  const reduced = model.status?.reduced;
  if (!reduced) return null;
  const i = model.index;
  if (reduced.reason === "firstIndex" && i.state === "indexing" && i.done < i.total) {
    // Only the first phase: the model is ready once every note has a vector.
    return `first index running, ${i.done.toLocaleString()} of ${i.total.toLocaleString()} notes`;
  }
  return reduced.message;
}

/** What doesn't work without a ready model (the rest of Hush does). */
export const MODEL_FEATURES = "related notes, search by meaning, tag suggestions, tag classifiers and Discover tags";
