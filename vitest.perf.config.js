// `npm run test:perf`: only the frame-budget checks (*.perf.test.ts), which `npm test`
// leaves out. Run them on an idle machine; on a busy one they fail for no reason.
import { configDefaults, defineConfig } from "vitest/config";
import viteConfig from "./vite.config.js";

export default defineConfig((env) => {
  const config = viteConfig(env);
  // Replaced, not merged: mergeConfig would append to the exclude list that leaves them out.
  return { ...config, test: { include: ["src/**/*.perf.test.ts"], exclude: configDefaults.exclude } };
});
