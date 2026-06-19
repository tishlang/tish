import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
// npm/vite-plugin-tish/test -> repo root
const repoRoot = path.resolve(here, "..", "..", "..");

// Resolve the `tish` binary the plugin should shell out to in tests. CI sets TISH_PATH; locally we
// fall back to the workspace build outputs, then to `tish` on PATH.
export function tishPath() {
  if (process.env.TISH_PATH && existsSync(process.env.TISH_PATH)) {
    return process.env.TISH_PATH;
  }
  const candidates = [
    path.join(repoRoot, "target", "release", "tish"),
    path.join(repoRoot, "target", "debug", "tish"),
  ];
  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  return "tish";
}

export const fixtureRoot = path.join(here, "fixtures", "hmr");
