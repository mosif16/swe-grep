import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, writeFile, chmod, readFile, rm, realpath } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { SweGrepBackend } from '../src/backend.js';

const exec = promisify(execFile);
test('tracked reference queries do not execute repository fsmonitor hooks', {
  skip: process.platform === 'win32' || !process.env.SWE_GREP_BINARY,
}, async (t) => {
  const temporary = await mkdtemp(path.join(os.tmpdir(), 'swe-git-policy-'));
  t.after(() => rm(temporary, {recursive:true, force:true}));
  const root = await realpath(temporary);
  await writeFile(path.join(root, 'app.rs'), 'pub fn target() {}\nfn caller() { target(); }\n');
  await exec('git', ['init', '--quiet'], {cwd:root});
  await exec('git', ['add', 'app.rs'], {cwd:root});
  const marker = path.join(root, 'hook-ran');
  const hook = path.join(root, 'monitor.sh');
  await writeFile(hook, '#!/bin/sh\nprintf invoked > "' + marker + '"\nexit 1\n');
  await chmod(hook, 0o755);
  await exec('git', ['config', 'core.fsmonitor', hook], {cwd:root});
  await exec('git', ['ls-files'], {cwd:root});
  assert.equal(await readFile(marker, 'utf8'), 'invoked', 'fixture must demonstrate the repository hook is active');
  await rm(marker);
  const backend = new SweGrepBackend({root, binary:process.env.SWE_GREP_BINARY});
  const result = await backend.call('swe_refs', {query:'target', trackedOnly:true, direction:'callers'});
  assert.ok(result.callers.length > 0);
  await assert.rejects(readFile(marker), {code:'ENOENT'});
});
