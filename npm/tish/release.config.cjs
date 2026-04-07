/**
 * When semantic-release runs with cwd `npm/tish`, cosmiconfig finds this file before
 * walking up to the repo root, so we avoid loading a stray `.releaserc.json` elsewhere.
 * Delegates to the repo-root `release.config.cjs` (CI env switches read-only vs full).
 */
module.exports = require("../../release.config.cjs");
