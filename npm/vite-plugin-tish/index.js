// Vite plugin for Tish (#284). Compiles each `.tish` file into Vite's module graph one module at a
// time via `tish compile-module`, so editing a leaf module hot-swaps without a full page reload —
// instead of the older out-of-band "compile the whole bundle and full-reload on every change" shim.
//
// Dev compiles request a source map (`tish compile-module --source-map`) so Vite's error overlay
// and the browser debugger resolve back to the original `.tish`.

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";

const TISH_EXT = ".tish";

/**
 * @param {object} [opts]
 * @param {string} [opts.tishPath]   Path to the `tish` binary (default: $TISH_PATH or `tish` on PATH).
 * @param {string} [opts.projectRoot] Root for resolving bare specifiers / node_modules (default: Vite root).
 * @param {"hmr"|"full-reload"} [opts.mode] `hmr` (default) hot-swaps modules; `full-reload` reloads the
 *        page on any `.tish` change (the documented fallback for `--target bytecode` apps).
 */
export default function tishPlugin(opts = {}) {
  const tishPath = opts.tishPath ?? process.env.TISH_PATH ?? "tish";
  const mode = opts.mode ?? "hmr";
  let projectRoot = opts.projectRoot;

  // Compile a single `.tish` file to one ES module. In `hmr` mode we request a source map and parse
  // the `{ js, map }` envelope; otherwise we take raw JS on stdout.
  function compileModule(file) {
    const args = [
      "compile-module",
      file,
      "--target",
      "js",
      "--format",
      "esm",
      "--vite-dev",
    ];
    args.push(mode === "hmr" ? "--source-map" : "--no-source-map");
    if (projectRoot) args.push("--project-root", projectRoot);
    const stdout = execFileSync(tishPath, args, {
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    });
    if (mode === "hmr") {
      const { js, map } = JSON.parse(stdout);
      return { code: js, map };
    }
    return { code: stdout, map: null };
  }

  return {
    name: "vite-plugin-tish",
    enforce: "pre",

    configResolved(config) {
      if (!projectRoot) projectRoot = config.root;
    },

    // Bring `.tish` imports into Vite's module graph: resolve relative `.tish` specifiers to absolute
    // file paths so Vite addresses each module and can invalidate it individually. Root-relative URLs
    // (`/src/x.tish`) and bare specifiers are left to Vite's resolver, which maps them to real files.
    resolveId(source, importer) {
      if (!source.endsWith(TISH_EXT)) return null;
      if (source.startsWith(".") && importer) {
        return path.resolve(path.dirname(importer.split("?")[0]), source);
      }
      if (path.isAbsolute(source) && existsSync(source)) {
        return source;
      }
      return null;
    },

    load(id) {
      const file = id.split("?")[0];
      if (!file.endsWith(TISH_EXT)) return null;
      const { code, map } = compileModule(file);
      if (mode !== "hmr") return { code, map };
      // Self-accepting boundary: editing this module re-runs it without reloading the page. Frameworks
      // (e.g. Lattish) can register their own `import.meta.hot.accept` handlers on top.
      const withBoundary = `${code}\nif (import.meta.hot) { import.meta.hot.accept(); }\n`;
      return { code: withBoundary, map };
    },

    handleHotUpdate(ctx) {
      const { file, server } = ctx;
      if (!file.endsWith(TISH_EXT)) return;
      if (mode === "full-reload") {
        server.ws.send({ type: "full-reload" });
        return [];
      }
      // Hand Vite the changed module node(s) so it performs per-module HMR rather than a full reload.
      const mods = server.moduleGraph.getModulesByFile(file);
      return mods ? [...mods] : undefined;
    },
  };
}
