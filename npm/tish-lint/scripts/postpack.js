#!/usr/bin/env node
// Remove the copies prepack made, so packing leaves nothing behind.
'use strict';
const path = require('path');
const { removePayload } = require('../../scripts/pack-payload');

removePayload(path.resolve(__dirname, '..'));
