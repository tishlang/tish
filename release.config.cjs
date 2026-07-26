/**
 * Semantic-release config (cosmiconfig name: "release").
 *
 * cosmiconfig prefers `.releaserc.json` over `release.config.cjs` in the same directory,
 * so the full plugin list lives in `release.full.config.json` (not auto-discovered).
 *
 * - TISH_SEMANTIC_RELEASE_CI=1: file:// repo + analyzer only (CI dry-runs, no git push / NPM_TOKEN).
 * - Otherwise: full config (npm + GitHub plugins, real remote).
 *
 * The commit-analyzer + release-notes-generator entries in release.full.config.json carry
 * a `parserOpts` that extends the bundled "angular" preset with the Conventional-Commits
 * `!` breaking shorthand (feat!:, fix(scope)!:). The stock angular header regex has no `!`,
 * so `feat!:` parsed as no-type -> no release. This is two regexes, not a whole preset
 * package. The dry-run (readOnlyCi) path REUSES those exact JSON entries so the version it
 * computes matches the real config — parserOpts lives in ONE place (the JSON).
 */
const path = require("path");
const { execSync } = require("child_process");

const full = require(path.join(__dirname, "release.full.config.json"));

function readOnlyCi() {
  const root = execSync("git rev-parse --show-toplevel", { encoding: "utf8" }).trim();
  const fileUrl = "file://" + root.replace(/\\/g, "/") + "/.git";
  // Reuse the analyzer + notes entries (with their parserOpts) straight from the full
  // config, so the dry-run version decision can't drift from the real one.
  const pick = (name) => full.plugins.find((p) => Array.isArray(p) && p[0] === name);
  return {
    branches: full.branches,
    repositoryUrl: fileUrl,
    plugins: [
      pick("@semantic-release/commit-analyzer"),
      pick("@semantic-release/release-notes-generator"),
    ],
  };
}

module.exports =
  process.env.TISH_SEMANTIC_RELEASE_CI === "1" ? readOnlyCi() : full;
