#!/usr/bin/env node
'use strict';
/**
 * Thin npm bin wrapper. Prefers:
 * 1) LAZYTREE_BIN
 * 2) cargo-built binary next to the repo / common install paths
 * 3) `lazytree` on PATH (native install)
 */
const { spawnSync } = require('child_process');
const fs = require('fs');
const path = require('path');

function candidates() {
  const out = [];
  if (process.env.LAZYTREE_BIN) out.push(process.env.LAZYTREE_BIN);
  const home = process.env.HOME || '';
  out.push(
    path.join(home, '.local', 'bin', 'lazytree'),
    path.join(home, '.cargo', 'bin', 'lazytree'),
    '/usr/local/bin/lazytree',
  );
  // Monorepo / linked package: ../../target/release/lazytree
  out.push(path.resolve(__dirname, '..', '..', 'target', 'release', 'lazytree'));
  out.push(path.resolve(__dirname, '..', '..', 'target', 'debug', 'lazytree'));
  out.push('lazytree');
  return out;
}

function resolveBin() {
  for (const c of candidates()) {
    if (c === 'lazytree') return c;
    try {
      fs.accessSync(c, fs.constants.X_OK);
      return c;
    } catch (_) {}
  }
  return null;
}

const bin = resolveBin();
if (!bin) {
  console.error(
    'lazytree native binary not found. Build with `cargo build --release` and ensure it is on PATH,\n' +
      'or set LAZYTREE_BIN=/path/to/lazytree',
  );
  process.exit(127);
}

const result = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit' });
process.exit(result.status == null ? 1 : result.status);
