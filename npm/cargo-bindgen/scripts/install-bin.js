#!/usr/bin/env node
'use strict';

/**
 * Copy platform/<os>-<arch>/tish-bindgen to bin/tish-bindgen so the npm bin is the native binary.
 * Runs on postinstall and before npm pack in CI.
 */

const fs = require('fs');
const path = require('path');

const platformKey = `${process.platform}-${process.arch}`;
const binaryName = process.platform === 'win32' ? 'tish-bindgen.exe' : 'tish-bindgen';

const root = path.join(__dirname, '..');
const src = path.join(root, 'platform', platformKey, binaryName);
const dest = path.join(root, 'bin', 'tish-bindgen');

if (!fs.existsSync(src)) {
  if (process.env.TISHLANG_CARGO_BINDGEN_NPM_PACK_REQUIRE === '1') {
    console.error(`[@tishlang/cargo-bindgen] No prebuilt binary for this platform: ${platformKey}`);
    console.error(`[@tishlang/cargo-bindgen] Expected: ${src}`);
    process.exit(1);
  }
  console.warn(
    `[@tishlang/cargo-bindgen] Skipping native bin (not found for ${platformKey}): ${src}`
  );
  console.warn(
    '[@tishlang/cargo-bindgen] From repo root run: ./npm/scripts/build-binaries.sh — then: node npm/cargo-bindgen/scripts/install-bin.js'
  );
  process.exit(0);
}

fs.mkdirSync(path.dirname(dest), { recursive: true });
fs.copyFileSync(src, dest);
try {
  fs.chmodSync(dest, 0o755);
} catch (_) {
  /* Windows may ignore chmod */
}
