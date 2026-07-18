// Vite plugin for Tish (#284). Compiles each `.tish` file into Vite's module graph one module at a
// time via `tish compile-module`, so editing a leaf module hot-swaps without a full page reload —
// instead of the older out-of-band "compile the whole bundle and full-reload on every change" shim.
//
// Dev compiles request a source map (`tish compile-module --source-map`) so Vite's error overlay
// and the browser debugger resolve back to the original `.tish`.
//
// Platform/surface cascade (RN-style `Button.macos.tish` / `.web.tish` / …) is owned by the `tish`
// CLI (`tish resolve-id`); this plugin must not reimplement those rules.

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
 * @param {string} [opts.platform] Platform token for resolve cascade (`macos`, `ios`, `web`, …).
 * @param {string} [opts.surface] Surface token (`native`, `webview`, `web`). Defaults from
 *        `TISH_PLATFORM` / `TISH_SURFACE` when unset.
 */
export default function tishPlugin(opts = {}) {
  const tishPath = opts.tishPath ?? process.env.TISH_PATH ?? "tish";
  const mode = opts.mode ?? "hmr";
  const platform = opts.platform ?? process.env.TISH_PLATFORM;
  const surface = opts.surface ?? process.env.TISH_SURFACE;
  let projectRoot = opts.projectRoot;
  /** @type {boolean|null} */
  let supportsPlatformFlags = null;

  function envForChild() {
    const env = { ...process.env };
    if (platform) env.TISH_PLATFORM = platform;
    if (surface) env.TISH_SURFACE = surface;
    return env;
  }

  function detectPlatformFlags() {
    if (supportsPlatformFlags !== null) return supportsPlatformFlags;
    try {
      const help = execFileSync(tishPath, ["compile-module", "--help"], {
        encoding: "utf8",
        env: envForChild(),
      });
      supportsPlatformFlags =
        help.includes("--platform") && help.includes("--surface");
    } catch {
      supportsPlatformFlags = false;
    }
    return supportsPlatformFlags;
  }

  function platformArgs() {
    if (!detectPlatformFlags()) return [];
    const args = [];
    if (platform) args.push("--platform", platform);
    if (surface) args.push("--surface", surface);
    return args;
  }

  function runTish(args, maxBuffer = 1024 * 1024) {
    return execFileSync(tishPath, args, {
      encoding: "utf8",
      maxBuffer,
      env: envForChild(),
    });
  }

  /** Resolve relative imports through `tish resolve-id` (same cascade as compile). */
  function resolveTishId(source, importer) {
    if (!detectPlatformFlags()) return null;
    const args = ["resolve-id", source, ...platformArgs()];
    if (importer) args.push("--importer", importer.split("?")[0]);
    try {
      const out = runTish(args).trim();
      return out || null;
    } catch {
      return null;
    }
  }

  function compileModule(file) {
    const args = [
      "compile-module",
      file,
      "--target",
      "js",
      "--format",
      "esm",
      "--vite-dev",
      ...platformArgs(),
    ];
    args.push(mode === "hmr" ? "--source-map" : "--no-source-map");
    if (projectRoot) args.push("--project-root", projectRoot);
    try {
      const stdout = runTish(args, 64 * 1024 * 1024);
      if (mode === "hmr") {
        const { js, map } = JSON.parse(stdout);
        return { code: js, map };
      }
      return { code: stdout, map: null };
    } catch (err) {
      const msg = String(err?.stderr || err?.message || err);
      // Older tish CLIs reject --platform; retry once without flags (env still set).
      if (
        supportsPlatformFlags !== false &&
        /unexpected argument '--platform'|unexpected argument '--surface'/.test(msg)
      ) {
        supportsPlatformFlags = false;
        return compileModule(file);
      }
      throw err;
    }
  }

  return {
    name: "vite-plugin-tish",
    enforce: "pre",

    configResolved(config) {
      if (!projectRoot) projectRoot = config.root;
    },

    resolveId(source, importer) {
      // Relative imports: always try platform cascade (with or without `.tish`).
      if (source.startsWith(".") && importer) {
        const resolved = resolveTishId(source, importer);
        if (resolved) return resolved;
        if (source.endsWith(TISH_EXT)) {
          return path.resolve(path.dirname(importer.split("?")[0]), source);
        }
        return null;
      }
      if (source.endsWith(TISH_EXT) && path.isAbsolute(source) && existsSync(source)) {
        return source;
      }
      return null;
    },

    load(id) {
      const file = id.split("?")[0];
      if (!file.endsWith(TISH_EXT)) return null;
      const { code, map } = compileModule(file);
      if (mode !== "hmr") return { code, map };
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
      const mods = server.moduleGraph.getModulesByFile(file);
      return mods ? [...mods] : undefined;
    },
  };
}
