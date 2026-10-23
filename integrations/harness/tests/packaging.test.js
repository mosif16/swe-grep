import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const manifest = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));

test('portable default export does not import optional native peers', async () => {
  const portable = await import('@recordandlearn/swe-grep-harness');
  assert.equal(typeof portable.SweGrepBackend, 'function');
  assert.equal(portable.apply, undefined);
});

test('GUI bundle resolves the actual native plugin subpath', async () => {
  const patch = await readFile(new URL('../cordis.patch.yml', import.meta.url), 'utf8');
  const entry = patch.split(String.fromCharCode(10)).find(line => line.trim().startsWith('name:'));
  const name = entry.trim().slice('name:'.length).trim().replaceAll("'", '');
  assert.equal(name, manifest.name + '/dsh');
  const plugin = await import(name);
  assert.equal(typeof plugin.apply, 'function');
  assert.deepEqual(plugin.inject, ['tools', 'workingDirectory', 'subprocess', 'fs']);
  assert.equal(manifest.dsh.bundle.patch, './cordis.patch.yml');
});

test('native peers are optional and package contains executable source entry', async () => {
  for (const peer of Object.keys(manifest.peerDependencies)) {
    assert.equal(manifest.peerDependenciesMeta[peer].optional, true);
  }
  const entry = manifest.bin['swe-grep-mcp'];
  const source = await readFile(new URL('../' + entry, import.meta.url), 'utf8');
  assert.ok(source.startsWith('#!/usr/bin/env node'));
  assert.ok(manifest.files.includes('src/'));
});
