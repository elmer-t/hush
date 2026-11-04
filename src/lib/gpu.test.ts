import { describe, expect, it } from "vitest";
import type { GpuStatus } from "$lib/api";
import { gpuText } from "$lib/gpu";

const rtx = { name: "NVIDIA GeForce RTX 4060 Ti", dedicatedBytes: 8 * 2 ** 30, discrete: true };
const base: GpuStatus = {
  adapters: [rtx],
  best: rtx,
  enabled: true,
  model: "bge-small-en-v1.5-q8",
  device: "cpu",
  reason: null,
  gpuModel: "BGE small English v1.5 (fp16, GPU)",
};

describe("gpuText", () => {
  it("names the GPU model to switch to when a separate GPU sits unused", () => {
    expect(gpuText(base)).toBe(
      "GPU: NVIDIA GeForce RTX 4060 Ti. bge-small-en-v1.5-q8 runs on the CPU (it isn't made for the GPU). " +
        "To index on the GPU, switch to BGE small English v1.5 (fp16, GPU) under Model above.",
    );
  });

  it("gives no hint when the GPU is turned off or there is no separate GPU", () => {
    expect(gpuText({ ...base, enabled: false })).not.toContain("switch to");
    expect(gpuText({ ...base, best: null })).not.toContain("switch to");
  });

  it("says why a GPU model stays on the CPU", () => {
    const s = { ...base, model: "bge-small-en-v1.5-fp16", reason: "turned off in Settings" };
    expect(gpuText(s)).toBe(
      "GPU: NVIDIA GeForce RTX 4060 Ti. bge-small-en-v1.5-fp16 runs on the CPU: turned off in Settings.",
    );
  });

  it("says when the model runs on the GPU", () => {
    expect(gpuText({ ...base, model: "bge-small-en-v1.5-fp16", device: "gpu" })).toBe(
      "GPU: NVIDIA GeForce RTX 4060 Ti. bge-small-en-v1.5-fp16 runs on the GPU.",
    );
  });
});
