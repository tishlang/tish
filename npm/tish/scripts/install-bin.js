#!/usr/bin/env node
'use strict';

/**
 * Copy platform/<os>-<arch>/tish to bin/tish so the npm bin is the native binary.
 * Runs on postinstall and before npm pack in CI.
 */

const fs = require('fs');
const path = require('path');

const platformKey = `${process.platform}-${process.arch}`;
const binaryName = process.platform === 'win32' ? 'tish.exe' : 'tish';

const root = path.join(__dirname, '..');
const src = path.join(root, 'platform', platformKey, binaryName);
const dest = path.join(root, 'bin', 'tish');

if (!fs.existsSync(src)) {
  if (process.env.TISH_NPM_PACK_REQUIRE === '1') {
    console.error(`[tish] No prebuilt binary for this platform: ${platformKey}`);
    console.error(`[tish] Expected: ${src}`);
    process.exit(1);
  }
  console.warn(`[tish] Skipping native bin (not found for ${platformKey}): ${src}`);
  console.warn('[tish] From repo root run: ./npm/scripts/build-binaries.sh — then: node npm/tish/scripts/install-bin.js');
  process.exit(0);
}

fs.mkdirSync(path.dirname(dest), { recursive: true });
fs.copyFileSync(src, dest);
try {
  fs.chmodSync(dest, 0o755);
} catch (_) {
  /* Windows may ignore chmod */
}
