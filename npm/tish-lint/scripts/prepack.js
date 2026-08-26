#!/usr/bin/env node
// Copy the Rust payload into the package before packing. See ../../scripts/pack-payload.js.
'use strict';
const path = require('path');
const { copyPayload } = require('../../scripts/pack-payload');

copyPayload({ pkgDir: path.resolve(__dirname, '..'), crate: 'tish_lint' });
