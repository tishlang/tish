/**
 * Semantic-release entrypoint (picked up before .releaserc.json).
 * - Local / full release: same as .releaserc.json (npm + GitHub plugins, real remote).
 * - CI version preview: TISH_SEMANTIC_RELEASE_CI=1 uses a local file:// repo + analyzer only
 *   so dry-runs never need git push or NPM_TOKEN (see workflow).
 */
const path = require("path");
const { execSync } = require("child_process");

const full = require(path.join(__dirname, ".releaserc.json"));

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
