import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, mkdir, symlink, rm, readFile, chmod, realpath } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { SweGrepBackend } from '../src/backend.js';

const fixture = fileURLToPath(new URL('../test/fixtures/fake-swe-grep.js', import.meta.url));
const ok = (value = { ok: true }) => ({ exitCode: 0, stdout: JSON.stringify(value), stderr: '' });
async function workspace(t) {
  const temporary = await mkdtemp(path.join(os.tmpdir(), 'swe-backend-'));
  t.after(() => rm(temporary, { recursive: true, force: true }));
  const root = await realpath(temporary);
  await mkdir(path.join(root, 'src'));
  await writeFile(path.join(root, 'src', 'code.js'), 'export function sample() { return 1; }\n');
  return root;
}
function rejectsCode(work, code) { return assert.rejects(work, error => error.code === code); }

test('validates all model input before filesystem or runner access', async () => {
  let touched = false;
  const backend = new SweGrepBackend({ runner: async () => { touched = true; return ok(); }, filesystem: { resolve() { touched = true; }, contains() { touched = true; } } });
  for (const [tool, args] of [
    ['unknown', { query: 'x' }], ['swe_search', {}], ['swe_search', { query: '😀'.repeat(65) }],
    ['swe_search', { query: 'x', rawargs: ['--help'] }], ['swe_search', { query: 'x', env: {} }],
    ['swe_search', { query: 'x', binary: '/bin/sh' }], ['swe_search', { query: 'x', root: '/' }],
    ['swe_search', { query: 'x', cacheDir: '/tmp' }], ['swe_search', { query: 'x', limit: 101 }],
    ['swe_search', { query: 'x', definitionsOnly: true, usagesOnly: true }],
  ]) await rejectsCode(() => backend.call(tool, args), 'SWE_GREP_INVALID_INPUT');
  assert.equal(touched, false);
});

test('rejects ambiguous relative binaries but accepts PATH and absolute binaries', () => {
  for (const binary of ['./swe-grep', 'bin/swe-grep', 'bin\\swe-grep', '']) assert.throws(() => new SweGrepBackend({ binary }), { code: 'SWE_GREP_INVALID_INPUT' });
  new SweGrepBackend({ binary: 'swe-grep' });
  new SweGrepBackend({ binary: process.execPath });
});

test('search uses controlled named arguments, bounds, flags, and root cwd', async t => {
  const root = await workspace(t);
  let request;
  const backend = new SweGrepBackend({ root, runner: async value => { request = value; return ok(); } });
  assert.deepEqual(await backend.call('swe_search', { query: '--help; $(evil)', path: 'src/code.js', language: 'typescript', mode: 'regex', limit: 7, exclude: ['--help', '*.lock'], pathGlob: ['src/**'], definitionsOnly: true }), { ok: true });
  assert.equal(request.cwd, root);
  assert.equal(request.argv[0], 'swe-grep');
  for (const flag of ['--no-config', '--disable-telemetry', 'search', '--symbol=--help; $(evil)', '--path=' + path.join(root, 'src/code.js'), '--json-only', '--concise', '--disable-fd', '--disable-ast-grep', '--mode=regex', '--top-hits=7', '--max-matches=7', '--language=typescript', '--exclude=--help', '--exclude=*.lock', '--path-glob=src/**', '--definitions-only']) assert.ok(request.argv.includes(flag), flag);
  assert.equal(request.maxOutputBytes, 2097152);
  assert.ok(request.signal instanceof AbortSignal);
});

test('search defaults, literal/text modes and usage filter', async t => {
  const root = await workspace(t);
  let argv;
  const backend = new SweGrepBackend({ root, runner: async request => { argv = request.argv; return ok(); } });
  await backend.call('swe_search', { query: 'x' });
  assert.ok(argv.includes('--mode=literal'));
  assert.ok(argv.includes('--top-hits=20'));
  await backend.call('swe_search', { query: 'x', mode: 'text', usagesOnly: true });
  assert.ok(argv.includes('--text'));
  assert.ok(argv.includes('--usages-only'));
  assert.ok(!argv.some(value => value.startsWith('--mode=')));
  await backend.call('swe_search', { query: 'sample', exactSymbol: true, definitionsOnly: true });
  assert.ok(argv.includes('--exact-symbol'));
});

test('context accepts directories only and concise is controllable', async t => {
  const root = await workspace(t);
  let argv;
  const backend = new SweGrepBackend({ root, runner: async request => { argv = request.argv; return ok(); } });
  await backend.call('swe_context', { query: '-x', path: 'src' });
  assert.ok(argv.includes('context'));
  assert.ok(argv.includes('--symbol=-x'));
  assert.ok(argv.includes('--concise'));
  assert.ok(!argv.includes('--callers'));
  assert.ok(!argv.includes('--callees'));
  await backend.call('swe_context', { query: 'x', concise: false });
  assert.ok(!argv.includes('--concise'));
  await rejectsCode(() => backend.call('swe_context', { query: 'x', path: 'src/code.js' }), 'SWE_GREP_INVALID_PATH');
});

test('context can select a search hit and explicitly request relationships', async t => {
  const root = await workspace(t);
  let argv;
  const backend = new SweGrepBackend({ root, runner: async request => { argv = request.argv; return ok(); } });
  for (const direction of ['callers', 'callees', 'both']) {
    await backend.call('swe_context', { query: 'sample', targetFile: 'src/code.js', targetLine: 230, language: 'javascript', direction });
    assert.ok(argv.includes('--target-file=src/code.js'));
    assert.ok(argv.includes('--target-line=230'));
    assert.ok(argv.includes('--language=javascript'));
    assert.equal(argv.includes('--callers'), direction !== 'callees');
    assert.equal(argv.includes('--callees'), direction !== 'callers');
  }
});

test('definition selectors retain the requested directory or file scope', async t => {
  const root = await workspace(t);
  const outside = await workspace(t);
  await symlink(path.join(outside, 'src/code.js'), path.join(root, 'src/escape.js'));
  let calls = 0;
  const backend = new SweGrepBackend({ root, runner: async () => { calls++; return ok(); } });
  for (const tool of ['swe_context', 'swe_refs']) {
    await rejectsCode(() => backend.call(tool, { query: 'sample', path: 'src', targetFile: 'escape.js' }), 'SWE_GREP_PATH_OUTSIDE_ROOT');
    await rejectsCode(() => backend.call(tool, { query: 'sample', targetFile: 'src' }), 'SWE_GREP_INVALID_PATH');
    await rejectsCode(() => backend.call(tool, { query: 'sample', targetFile: 'missing.js' }), 'SWE_GREP_INVALID_PATH');
  }
  await writeFile(path.join(root, 'src/other.js'), 'function sample() {}\n');
  await rejectsCode(() => backend.call('swe_refs', { query: 'sample', path: 'src/code.js', targetFile: 'other.js' }), 'SWE_GREP_PATH_OUTSIDE_ROOT');
  assert.equal(calls, 0);
  await backend.call('swe_refs', { query: 'sample', path: 'src/code.js', targetFile: 'code.js', targetLine: 1 });
  assert.equal(calls, 1);
});

test('refs accepts file scope, directions and filters', async t => {
  const root = await workspace(t);
  let argv;
  const backend = new SweGrepBackend({ root, runner: async request => { argv = request.argv; return ok(); } });
  for (const direction of ['callers', 'callees', 'both']) {
    await backend.call('swe_refs', { query: '--callers', path: 'src/code.js', direction, exclude: ['*.lock'], pathGlob: ['src/**'], excludeTests: true, trackedOnly: true });
    assert.equal(argv.includes('--callers'), direction !== 'callees');
    assert.equal(argv.includes('--callees'), direction !== 'callers');
    for (const flag of ['--format=json', '--symbol=--callers', '--exclude=*.lock', '--path-glob=src/**', '--exclude-tests', '--tracked-only']) assert.ok(argv.includes(flag));
  }
});

test('canonical confinement rejects traversal, sibling-prefix and symlink escapes', async t => {
  const root = await workspace(t);
  const outside = await workspace(t);
  await symlink(outside, path.join(root, 'escape'), 'dir');
  const backend = new SweGrepBackend({ root, runner: async () => assert.fail('must not execute') });
  await mkdir(path.join(root, 'src-copy'));
  const nested = new SweGrepBackend({ root: path.join(root, 'src'), runner: async () => assert.fail('sibling prefix is not containment') });
  await rejectsCode(() => nested.call('swe_search', { query: 'x', path: '../src-copy' }), 'SWE_GREP_PATH_OUTSIDE_ROOT');
  for (const value of ['..', outside, 'escape', 'escape/src/code.js']) await rejectsCode(() => backend.call('swe_search', { query: 'x', path: value }), 'SWE_GREP_PATH_OUTSIDE_ROOT');
  await rejectsCode(() => backend.call('swe_search', { query: 'x', path: 'missing' }), 'SWE_GREP_INVALID_PATH');
});

test('without configured root current cwd is the confinement boundary', async t => {
  const root = await workspace(t);
  const backend = new SweGrepBackend({ runner: async request => ok({ cwd: request.cwd }) });
  assert.deepEqual(await backend.call('swe_search', { query: 'x' }, { cwd: root }), { cwd: root });
  await rejectsCode(() => backend.call('swe_search', { query: 'x', path: '..' }, { cwd: root }), 'SWE_GREP_PATH_OUTSIDE_ROOT');
});

test('injected filesystem retains opaque identity and controls containment', async () => {
  const parent = { path: '/virtual/root', kind: 'directory', identity: Symbol() };
  const child = { path: '/virtual/root/code', kind: 'file', identity: Symbol() };
  let calls = 0;
  const filesystem = { resolve(value, cwd, signal) { assert.ok(signal instanceof AbortSignal); return calls++ === 0 ? parent : child; }, contains(a, b) { assert.equal(a, parent); assert.equal(b, child); return false; } };
  const backend = new SweGrepBackend({ root: '/virtual/root', filesystem, runner: async () => assert.fail('host policy denied') });
  await rejectsCode(() => backend.call('swe_search', { query: 'x' }), 'SWE_GREP_PATH_OUTSIDE_ROOT');
});

test('JSON, exit, truncation and spawn failures have stable codes', async t => {
  const root = await workspace(t);
  for (const [result, code] of [
    [{ ...ok(), stdout: '{' }, 'SWE_GREP_INVALID_OUTPUT'],
    [{ ...ok(), stdout: '[]' }, 'SWE_GREP_INVALID_OUTPUT'],
    [{ ...ok(), stdout: 'null' }, 'SWE_GREP_INVALID_OUTPUT'],
    [{ ...ok(), stdout: '{}\n{}' }, 'SWE_GREP_INVALID_OUTPUT'],
    [{ ...ok(), stdoutTruncated: true }, 'SWE_GREP_OUTPUT_TRUNCATED'],
    [{ ...ok(), stdout: ' '.repeat(129) }, 'SWE_GREP_OUTPUT_TRUNCATED'],
    [{ ...ok(), exitCode: 2, stdout: '{"error":"bad"}' }, 'SWE_GREP_CLI_ERROR'],
    [{ ...ok(), exitCode: 2, stdout: '{' }, 'SWE_GREP_CLI_ERROR'],
    [{ ...ok(), stdout: '{"error":"bad"}' }, 'SWE_GREP_CLI_ERROR'],
    [{ ...ok(), stdout: '{"hits":[],"errors":["tool failed"]}' }, 'SWE_GREP_CLI_ERROR'],
    [{ ...ok(), signal: 'SIGTERM', exitCode: null }, 'SWE_GREP_CLI_ERROR'],
  ]) await rejectsCode(() => new SweGrepBackend({ root, maxOutputBytes: 128, runner: async () => result }).call('swe_search', { query: 'x' }), code);
  await rejectsCode(() => new SweGrepBackend({ root, binary: path.join(root, 'no-such-binary') }).call('swe_search', { query: 'x' }), 'SWE_GREP_BINARY_NOT_FOUND');
});

test('total timeout covers filesystem resolution and caller abort is merged', async t => {
  const root = await workspace(t);
  const backend = new SweGrepBackend({ timeoutMs: 20, filesystem: { resolve: () => new Promise(() => {}), contains: () => true } });
  await rejectsCode(() => backend.call('swe_search', { query: 'x' }), 'SWE_GREP_TIMEOUT');
  const controller = new AbortController();
  controller.abort();
  await rejectsCode(() => new SweGrepBackend({ root }).call('swe_search', { query: 'x' }, { signal: controller.signal }), 'SWE_GREP_ABORTED');
  const active = new AbortController();
  await rejectsCode(() => new SweGrepBackend({ root, runner: async ({ signal }) => { active.abort(); assert.equal(signal.aborted, true); return ok(); } }).call('swe_search', { query: 'x' }, { signal: active.signal }), 'SWE_GREP_ABORTED');
});

test('deadline timer and caller listener are removed on successful completion', async t => {
  const root = await workspace(t);
  let merged;
  const controller = new AbortController();
  await new SweGrepBackend({ root, timeoutMs: 20, runner: async ({ signal }) => { merged = signal; return ok(); } }).call('swe_search', { query: 'x' }, { signal: controller.signal });
  controller.abort();
  await new Promise(resolve => setTimeout(resolve, 35));
  assert.equal(merged.aborted, false);
});

test('local spawn runs shell-free, bounds diagnostics and rejects overflowing stdout', async t => {
  const root = await workspace(t);
  await chmod(fixture, 0o755);
  const backend = new SweGrepBackend({ root, binary: fixture, maxOutputBytes: 1024 });
  const result = await backend.call('swe_search', { query: '$(touch SHOULD_NOT_EXIST)' });
  assert.ok(result.argv.includes('--symbol=$(touch SHOULD_NOT_EXIST)'));
  assert.equal(result.cwd, root);
  assert.deepEqual(await backend.call('swe_search', { query: 'diagnostics' }), { ok: true });
  await rejectsCode(() => backend.call('swe_search', { query: 'flood' }), 'SWE_GREP_OUTPUT_TRUNCATED');
  await rejectsCode(() => backend.call('swe_search', { query: 'invalid' }), 'SWE_GREP_INVALID_OUTPUT');
  await rejectsCode(() => backend.call('swe_search', { query: 'error' }), 'SWE_GREP_CLI_ERROR');
});

for (const trigger of ['timeout', 'abort']) test('local ' + trigger + ' awaits close and kills stubborn process group', { skip: process.platform === 'win32' }, async t => {
  const root = await workspace(t);
  await chmod(fixture, 0o755);
  const controller = new AbortController();
  const backend = new SweGrepBackend({ root, binary: fixture, timeoutMs: trigger === 'timeout' ? 400 : 10000 });
  const running = backend.call('swe_search', { query: 'tree' }, { signal: controller.signal });
  const checked = rejectsCode(() => running, trigger === 'timeout' ? 'SWE_GREP_TIMEOUT' : 'SWE_GREP_ABORTED');
  if (trigger === 'abort') {
    // Wait for readiness via file IO rather than guessing when child spawn finished.
    for (let i = 0; ; i++) {
      try { await readFile(path.join(root, 'processes.json')); break; }
      catch (error) { if (error.code !== 'ENOENT' || i > 200) throw error; await new Promise(resolve => setTimeout(resolve, 5)); }
    }
    controller.abort();
  }
  await checked;
  const pids = JSON.parse(await readFile(path.join(root, 'processes.json'), 'utf8'));
  assert.throws(() => process.kill(pids.parent, 0), { code: 'ESRCH' });
  // The grandchild is reaped by init asynchronously on some platforms.
  for (let i = 0; ; i++) {
    try { process.kill(pids.child, 0); }
    catch (error) { assert.equal(error.code, 'ESRCH'); break; }
    assert.ok(i < 200, 'grandchild must be killed and reaped');
    await new Promise(resolve => setTimeout(resolve, 5));
  }
});

test('optional real CLI integration search context and refs', { skip: !process.env.SWE_GREP_BINARY && 'Set SWE_GREP_BINARY to test an installed CLI' }, async t => {
  const root = await workspace(t);
  const backend = new SweGrepBackend({ root, binary: process.env.SWE_GREP_BINARY, timeoutMs: 30000 });
  for (const name of ['swe_search', 'swe_context', 'swe_refs']) {
    let result;
    try { result = await backend.call(name, { query: 'sample' }); }
    catch (error) { if (error.code === 'SWE_GREP_BINARY_NOT_FOUND') return t.skip('SWE_GREP_BINARY is unavailable'); throw error; }
    assert.ok(result && typeof result === 'object' && !Array.isArray(result));
    if (name === 'swe_search') assert.ok(result.hits.some(hit => hit.snippet.includes('sample')));
    else assert.equal(result.symbol, 'sample');
  }
  for (const name of ['swe_search', 'swe_refs']) {
    const result = await backend.call(name, { query: 'sample', path: 'src/code.js' });
    assert.ok(result && typeof result === 'object');
  }
});
