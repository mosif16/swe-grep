import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { validateJsonSchemaValue, ToolArgsError } from '@deepseek-ai/dsh-tools';
import { apply, Config, inject, name } from '../src/dsh.js';
import { TOOLS } from '../src/contracts.js';

function fixture(config = {}, behavior = {}) {
  const registered = new Map();
  const calls = { ensure: [], resolve: [], stat: [], contains: [], lookup: [], spawn: [], reads: [], waits: 0 };
  const targets = new Map();
  const facts = new WeakMap();
  function targetFor(value) {
    if (!targets.has(value)) {
      const target = { targetKey: Object.freeze({ opaque: value }), displayPath: 'display-only' };
      targets.set(value, target);
      facts.set(target, { path: value, type: value.endsWith('.rs') ? 'file' : 'directory' });
    }
    return targets.get(value);
  }
  const ctx = {
    tools: { register(definition) { registered.set(definition.name, definition); } },
    workingDirectory: { async ensure(agent, signal) {
      calls.ensure.push({ agent, signal }); signal?.throwIfAborted(); return '/execution/repo';
    } },
    fs: {
      async resolve(value, options) {
        calls.resolve.push({ value, options }); options.signal?.throwIfAborted();
        let canonical = path.posix.resolve(options.cwd, value);
        if (canonical === '/execution/repo/link') canonical = '/execution/outside';
        return targetFor(canonical);
      },
      async stat(target, signal) { calls.stat.push({ target, signal }); return facts.get(target); },
      processPath(target) { assert.ok(facts.has(target)); return facts.get(target).path; },
      contains(root, target) {
        calls.contains.push({ root, target });
        assert.ok(facts.has(root)); assert.ok(facts.has(target));
        const relative = path.posix.relative(facts.get(root).path, facts.get(target).path);
        return relative === '' || (!relative.startsWith('..') && !path.posix.isAbsolute(relative));
      },
    },
    subprocess: {
      async resolveExecutable(command, env, signal) {
        calls.lookup.push({ command, env, signal }); signal.throwIfAborted();
        if (behavior.lookupError) throw behavior.lookupError;
        return '/execution/bin/swe-grep';
      },
      spawn(spec) {
        calls.spawn.push(spec);
        const output = behavior.output ?? { matches: [] };
        const done = behavior.done ? behavior.done(spec) : Promise.resolve({ exitCode: 0, signal: null });
        return {
          done,
          async waitForExit() { calls.waits++; if (behavior.wait) await behavior.wait(); return true; },
          collected: Object.fromEntries(['stdout', 'stderr'].map(stream => [stream, {
            readFrom(offset) {
              calls.reads.push({ stream, offset });
              return { text: stream === 'stdout' ? JSON.stringify(output) : '', nextOffset: 0,
                lossy: stream === 'stdout' ? !!behavior.stdoutLossy : !!behavior.stderrLossy };
            },
          }])),
        };
      },
    },
  };
  apply(ctx, config);
  const agent = { session: { id: 'fake-session' } };
  const controller = new AbortController();
  const exec = { agent, signal: controller.signal };
  return { registered, calls, agent, controller, exec,
    call: (tool = 'swe_search', args = { query: 'Widget' }, execution = exec) => registered.get(tool).execute(args, execution) };
}

test('exports real Schemastery config with validated defaults', () => {
  assert.equal(name, 'swe-grep');
  assert.deepEqual(inject, ['tools', 'workingDirectory', 'subprocess', 'fs']);
  assert.equal(Config({}).timeoutMs, 30000);
  assert.equal(Config({}).maxOutputBytes, 2097152);
  for (const config of [{ timeoutMs: 0 }, { timeoutMs: 0.5 }, { maxOutputBytes: -1 }, { binary: '' }, { root: '' }]) {
    assert.throws(() => Config(config));
  }
});

test('actual defineTool registers canonical schemas, JSON renderers, exclusive dispatch', async () => {
  const f = fixture();
  assert.deepEqual([...f.registered.keys()], TOOLS.map(tool => tool.name));
  for (const tool of TOOLS) {
    const definition = f.registered.get(tool.name);
    assert.deepEqual(definition.parameters, tool.inputSchema);
    assert.equal(definition.output.schema.type, 'object');
    assert.deepEqual(validateJsonSchemaValue(definition.output.schema, { matches: [] }, ''), []);
    assert.deepEqual(definition.output.render({}, { matches: [] }), [{ type: 'text', text: JSON.stringify({ matches: [] }) }]);
    assert.equal(definition.isConcurrencySafe?.({ query: 'Widget' }) ?? false, false);
    assert.equal(definition.timeoutMs, undefined);
    assert.deepEqual(await f.call(tool.name), { matches: [] });
  }
});

test('SDK validation and strict canonical root reject malformed input before services', async () => {
  const f = fixture();
  for (const args of [{}, { query: 2 }, { query: 'Widget', extra: true }, { query: 'Widget', limit: '2' }]) {
    await assert.rejects(f.call('swe_search', args), error => error instanceof ToolArgsError);
  }
  assert.equal(f.calls.ensure.length, 0);
  assert.equal(f.calls.spawn.length, 0);
});

test('native calls use agent cwd, canonical identities, executable lookup and managed collection', async () => {
  const f = fixture({ binary: '/configured/swe-grep', maxOutputBytes: 4096 });
  assert.deepEqual(await f.call(), { matches: [] });
  assert.deepEqual(f.calls.ensure, [{ agent: f.agent, signal: f.exec.signal }]);
  assert.equal(f.calls.lookup[0].command, '/configured/swe-grep');
  const spec = f.calls.spawn[0];
  assert.equal(spec.argv[0], '/execution/bin/swe-grep');
  assert.equal(spec.cwd, '/execution/repo');
  assert.ok(spec.argv.includes('--path=/execution/repo'));
  assert.ok(spec.signal instanceof AbortSignal);
  assert.deepEqual(spec.stdio, { stdin: 'ignore', stdout: { maxBytes: 4096 }, stderr: { maxBytes: 4096 } });
  assert.equal(spec.graceMs, 1000);
  assert.deepEqual(spec.env, { SWE_GREP_DISABLE_TELEMETRY: '1', GIT_OPTIONAL_LOCKS: '0', GIT_CONFIG_COUNT: '1', GIT_CONFIG_KEY_0: 'core.fsmonitor', GIT_CONFIG_VALUE_0: 'false' });
  assert.deepEqual(f.calls.reads, [{ stream: 'stdout', offset: 0 }, { stream: 'stderr', offset: 0 }]);
  assert.equal(f.calls.waits, 1);
  assert.equal(f.calls.stat.length, 2);
  assert.equal(f.calls.contains.length, 1);
});

test('configured root is session-relative and symlink escape uses provider containment', async () => {
  const f = fixture({ root: 'src' });
  await f.call();
  assert.equal(f.calls.spawn[0].cwd, '/execution/repo/src');
  const escaped = fixture();
  await assert.rejects(escaped.call('swe_search', { query: 'Widget', path: 'link' }), { code: 'SWE_GREP_PATH_OUTSIDE_ROOT' });
  assert.equal(escaped.calls.spawn.length, 0);
});

test('native definition selectors retain provider identities and analysis scope', async () => {
  const f = fixture();
  await f.call('swe_context', { query: 'Widget', path: 'src', targetFile: 'widget.rs', targetLine: 120, language: 'rust', direction: 'both' });
  assert.equal(f.calls.resolve[2].options.cwd, '/execution/repo/src');
  assert.equal(f.calls.contains.length, 2);
  assert.equal(f.calls.contains[1].root, f.calls.stat[1].target);
  assert.equal(f.calls.contains[1].target, f.calls.stat[2].target);
  for (const flag of ['--target-file=widget.rs', '--target-line=120', '--language=rust', '--callers', '--callees']) {
    assert.ok(f.calls.spawn[0].argv.includes(flag), flag);
  }
  const escaped = fixture();
  await assert.rejects(escaped.call('swe_context', { query: 'Widget', targetFile: 'link' }), { code: 'SWE_GREP_INVALID_PATH' });
  assert.equal(escaped.calls.spawn.length, 0);
});

test('context file paths and agent-less execution fail before spawn', async () => {
  const f = fixture();
  await assert.rejects(f.call('swe_context', { query: 'Widget', path: 'file.rs' }), { code: 'SWE_GREP_INVALID_PATH' });
  await assert.rejects(f.call('swe_search', { query: 'Widget' }, { signal: f.exec.signal }), /agent Session/);
  assert.equal(f.calls.spawn.length, 0);
});

test('pre-aborted calls do not create subprocesses', async () => {
  const f = fixture();
  f.controller.abort(new Error('cancelled'));
  await assert.rejects(f.call(), /cancelled/);
  assert.equal(f.calls.spawn.length, 0);
});

test('lossy collected stdout is never parsed as a complete success', async () => {
  const f = fixture({}, { stdoutLossy: true });
  await assert.rejects(f.call(), { code: 'SWE_GREP_OUTPUT_TRUNCATED' });
  assert.equal(f.calls.reads.length, 2);
});

test('provider outcome rejection still awaits managed range cleanup', async () => {
  let cleaned = false;
  const f = fixture({}, {
    done: () => Promise.reject(new Error('provider failed')),
    async wait() { cleaned = true; },
  });
  await assert.rejects(f.call(), { code: 'SWE_GREP_EXECUTION_FAILED' });
  assert.equal(cleaned, true);
});

test('execution-world lookup errors propagate without a shell fallback', async () => {
  const f = fixture({}, { lookupError: Object.assign(new Error('missing'), { code: 'ENOENT' }) });
  await assert.rejects(f.call(), { code: 'SWE_GREP_BINARY_NOT_FOUND' });
  assert.equal(f.calls.spawn.length, 0);
});

for (const mode of ['caller cancellation', 'backend deadline']) {
  test(mode + ' forwards abort and awaits managed teardown', async () => {
    let spawned;
    const started = new Promise(resolve => { spawned = resolve; });
    let quiescent = false;
    const f = fixture({ timeoutMs: mode === 'backend deadline' ? 15 : 1000 }, {
      done(spec) {
        spawned();
        return new Promise(resolve => {
          spec.signal.addEventListener('abort', () => {
            setTimeout(() => resolve({ exitCode: null, signal: 'SIGTERM' }), 5);
          }, { once: true });
        });
      },
      async wait() { await new Promise(resolve => setTimeout(resolve, 5)); quiescent = true; },
    });
    const pending = f.call();
    await started;
    if (mode === 'caller cancellation') f.controller.abort();
    await assert.rejects(pending, { code: mode === 'backend deadline' ? 'SWE_GREP_TIMEOUT' : 'SWE_GREP_ABORTED' });
    assert.equal(f.calls.spawn[0].signal.aborted, true);
    assert.equal(quiescent, true, 'the backend must not abandon an injected managed runner');
  });
}
