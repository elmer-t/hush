import type { GpuStatus } from "$lib/api";

/** Settings → Indexing's line: which GPU there is, where the vault's model runs and,
 * when a separate GPU sits unused, which model to pick to use it. */
export function gpuText(s: GpuStatus): string {
  const found = s.best
    ? `GPU: ${s.best.name}.`
    : s.adapters.length > 0
      ? `No separate GPU found (${s.adapters.map((a) => a.name).join(", ")} shares the computer's memory).`
      : "No GPU found.";
  if (!s.model || !s.device) return found;
  if (s.device === "gpu") return `${found} ${s.model} runs on the GPU.`;
  if (s.reason) return `${found} ${s.model} runs on the CPU: ${s.reason}.`;
  const hint =
    s.best && s.enabled && s.gpuModel
      ? ` To index on the GPU, switch to ${s.gpuModel} under Model above.`
      : "";
  return `${found} ${s.model} runs on the CPU (it isn't made for the GPU).${hint}`;
}
