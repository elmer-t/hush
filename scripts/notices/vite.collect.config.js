// The app's Vite config plus a plugin that lists what goes into each bundle, for
// generate.mjs. Lines go to $COLLECT_OUT as "<environment>\t<module id>" for code and
// "<environment>\tasset\t<source file>" for assets (fonts, images). SvelteKit runs the
// browser build from this same config file, so it is listed too (as "client").
import { appendFileSync } from "node:fs";
import base from "../../vite.config.js";

export default (env) => {
  const config = base(env);
  const collect = {
    name: "hush-collect-bundled-files",
    generateBundle(_options, bundle) {
      const lines = [];
      for (const out of Object.values(bundle)) {
        for (const file of out.originalFileNames ?? []) lines.push(`${this.environment?.name}\tasset\t${file}`);
        if (out.type === "chunk") for (const id of out.moduleIds ?? Object.keys(out.modules ?? {})) lines.push(`${this.environment?.name}\t${id}`);
      }
      appendFileSync(process.env.COLLECT_OUT, lines.map((l) => `${l}\n`).join(""));
    },
  };
  return { ...config, plugins: [...config.plugins, collect] };
};
