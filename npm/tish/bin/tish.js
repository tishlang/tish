#!/usr/bin/env node
'use strict';

const path = require('path');
const fs = require('fs');
const { spawnSync } = require('child_process');

const platformKey = `${process.platform}-${process.arch}`;
const supported = ['darwin-arm64', 'darwin-x64', 'linux-x64', 'linux-arm64', 'win32-x64'];
const binaryName = process.platform === 'win32' ? 'tish.exe' : 'tish';

const binPath = path.join(__dirname, '..', 'platform', platformKey, binaryName);
if (!fs.existsSync(binPath)) {
  console.error(`[tish] Unsupported or missing binary for platform: ${platformKey}`);
  console.error('Supported:', supported.join(', '));
  console.error('Build from source: https://github.com/tish-lang/tish');
  process.exit(1);
}

// Convenience: "npx @tishlang/tish FILE [args]" → "tish run FILE [args]"
const args = process.argv.slice(2);
const subcommands = ['run', 'repl', 'compile', 'dump-ast'];
const first = args[0];
const looksLikeFile = first && !first.startsWith('-') && !subcommands.includes(first);

const finalArgs = looksLikeFile ? ['run', ...args] : args;

const result = spawnSync(binPath, finalArgs, {
  stdio: 'inherit',
  windowsHide: true,
});
process.exit(result.status !== null ? result.status : 1);
