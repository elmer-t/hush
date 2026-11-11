// Text for Settings → Storage → Compact vault (see `db::compact` in the backend).
import type { VaultSpace } from "$lib/api";
import { formatSize } from "$lib/size";

/** `0:07`, `12:05`, `1:02:03`: how long a compaction has been running. */
export function elapsedText(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** What compacting would do now, or why it isn't offered. */
export function compactNote(space: VaultSpace): string {
  const by = space.incremental ? " Space freed by emptying the trash is given back right away." : "";
  if (space.worthCompacting) return `Compacting gives back ${formatSize(space.reclaimableBytes)}.${by}`;
  if (space.reclaimableBytes === 0) return `Nothing to give back.${by}`;
  return (
    `Only ${formatSize(space.reclaimableBytes)} can be given back; ` +
    `compacting starts to pay off from 10 MB and 1% of the file.${by}`
  );
}
