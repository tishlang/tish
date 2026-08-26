#!/usr/bin/env node
// Remove the copies prepack made, so packing leaves nothing behind.
//
// Local dev may have had symlinks here instead; `scripts/dev-setup.sh` in the chuggie repo recreates
// them. Nothing about a release depends on them.
'use strict';
const fs = require('fs');
const path = require('path');

const here = path.resolve(__dirname, '..');
for (const name of ['Cargo.toml', 'crates', 'LICENSE', 'justfile']) {
  fs.rmSync(path.join(here, name), { recursive: true, force: true });
}
console.log('postpack: removed the copied payload');
