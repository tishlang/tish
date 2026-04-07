/**
 * Semantic-release config (cosmiconfig name: "release").
 *
 * cosmiconfig prefers `.releaserc.json` over `release.config.cjs` in the same directory,
 * so the full plugin list lives in `release.full.config.json` (not auto-discovered).
 *
 * - TISH_SEMANTIC_RELEASE_CI=1: file:// repo + analyzer only (CI dry-runs, no git push / NPM_TOKEN).
 * - Otherwise: full config (npm + GitHub plugins, real remote).
 */
const path = require("path");
const { execSync } = require("child_process");

const full = require(path.join(__dirname, "release.full.config.json"));

function readOnlyCi() {
  const root = execSync("git rev-parse --show-toplevel", { encoding: "utf8" }).trim();
  const fileUrl = "file://" + root.replace(/\\/g, "/") + "/.git";
  return {
    branches: full.branches,
    repositoryUrl: fileUrl,
    plugins: [
      "@semantic-release/commit-analyzer",
      "@semantic-release/release-notes-generator",
    ],
  };
}

module.exports =
  process.env.TISH_SEMANTIC_RELEASE_CI === "1" ? readOnlyCi() : full;
