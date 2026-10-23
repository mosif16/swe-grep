import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, writeFile, chmod, rm} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {SweGrepBackend} from '../src/backend.js';

test('portable child receives launch essentials, not credentials or harness controls', {skip:process.platform === 'win32'}, async t => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'swe-env-'));
  t.after(() => rm(root, {recursive:true, force:true}));
  const keys = ['OPENAI_API_KEY', 'DSH_REVIEW_MARKER', 'NODE_OPTIONS', 'GIT_DIR'];
  const old = Object.fromEntries(keys.map(key => [key, process.env[key]]));
  t.after(() => { for(const key of keys) { if(old[key] === undefined) delete process.env[key]; else process.env[key] = old[key]; } });
  process.env.OPENAI_API_KEY = 'synthetic-not-a-real-secret';
  process.env.DSH_REVIEW_MARKER = 'must-not-pass';
  process.env.NODE_OPTIONS = '--no-warnings';
  process.env.GIT_DIR = '/unrelated/repository';
  const binary = path.join(root, 'environment-probe');
  await writeFile(binary, '#!/usr/bin/env node\nconsole.log(JSON.stringify({leaks:Object.keys(process.env).filter(key=>["OPENAI_API_KEY","DSH_REVIEW_MARKER","NODE_OPTIONS","GIT_DIR"].includes(key)),path:!!process.env.PATH,monitor:process.env.GIT_CONFIG_VALUE_0}));\n');
  await chmod(binary, 0o755);
  const result = await new SweGrepBackend({root,binary}).call('swe_search', {query:'safe'});
  assert.deepEqual(result.leaks, []);
  assert.equal(result.path, true);
  assert.equal(result.monitor, 'false');
});
