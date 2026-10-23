import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js';
import { createMcpServer } from '../src/mcp.js';
import { TOOLS } from '../src/contracts.js';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const entry = fileURLToPath(new URL('../src/mcp.js', import.meta.url));

async function connected(t, backend) {
  const server = createMcpServer({ root }, { backend });
  const client = new Client({ name: 'mcp-test', version: '1.0.0' });
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  t.after(async () => { await client.close(); await server.close(); });
  await server.connect(serverTransport);
  await client.connect(clientTransport);
  return { server, client, clientTransport };
}

const search = { query: 'fixture' };

test('official client initializes, lists exact contracts and calls each tool', async t => {
  const calls = [];
  const { client } = await connected(t, { async call(name, args, options) {
    calls.push({ name, args, options });
    return { name, result: [] };
  } });
  assert.equal(client.getServerVersion().name, 'swe-grep');
  assert.ok(client.getServerCapabilities().tools);
  const listed = await client.listTools();
  assert.deepEqual(listed.tools.map(({ annotations, ...tool }) => tool), TOOLS);
  for (const tool of listed.tools) {
    assert.deepEqual(tool.annotations, {
      readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false,
    });
    const args = search;
    const result = await client.callTool({ name: tool.name, arguments: args });
    assert.deepEqual(result.structuredContent, { name: tool.name, result: [] });
    assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
    assert.notEqual(result.isError, true);
  }
  assert.equal(calls.length, 3);
  assert.ok(calls.every(call => call.options.signal instanceof AbortSignal));
});

test('unknown names and invalid input are explicit errors without backend execution', async t => {
  let calls = 0;
  const { client } = await connected(t, { async call() { calls++; return {}; } });
  for (const [name, args] of [
    ['missing', {}], ['swe_search', {}], ['swe_search', { query: 42 }],
    ['swe_search', { ...search, root: '/tmp' }],
    ['swe_search', { ...search, binary: 'other' }],
    ['swe_search', { ...search, cache: false }],
  ]) {
    await assert.rejects(client.callTool({ name, arguments: args }), error => error.code === -32602);
  }
  assert.equal(calls, 0);
});

test('backend errors have matching structured and text results with isError', async t => {
  const { client } = await connected(t, { async call() {
    throw Object.assign(new Error('fixture backend failed'), { code: 'FIXTURE_FAILURE' });
  } });
  const result = await client.callTool({ name: 'swe_search', arguments: search });
  assert.equal(result.isError, true);
  assert.deepEqual(result.structuredContent, {
    error: { code: 'FIXTURE_FAILURE', message: 'fixture backend failed' },
  });
  assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
});

function waitingBackend() {
  let started;
  let cancelled;
  const ready = new Promise(resolve => { started = resolve; });
  const aborted = new Promise(resolve => { cancelled = resolve; });
  return { ready, aborted, async call(name, args, { signal }) {
    started();
    return new Promise((resolve, reject) => {
      const abort = () => { cancelled(); reject(signal.reason); };
      if (signal.aborted) abort();
      else signal.addEventListener('abort', abort, { once: true });
    });
  } };
}

test('official client cancellation reaches backend signal', { timeout: 5000 }, async t => {
  const backend = waitingBackend();
  const { client } = await connected(t, backend);
  const controller = new AbortController();
  const request = client.callTool({ name: 'swe_search', arguments: search }, undefined, { signal: controller.signal });
  const rejected = assert.rejects(request);
  await backend.ready;
  controller.abort();
  await Promise.all([rejected, backend.aborted]);
});

for (const mode of ['close', 'disconnect']) {
  test(mode + ' cancels active backend calls', { timeout: 5000 }, async t => {
    const backend = waitingBackend();
    const { client, server, clientTransport } = await connected(t, backend);
    const request = client.callTool({ name: 'swe_search', arguments: search });
    const rejected = assert.rejects(request);
    await backend.ready;
    if (mode === 'close') await server.close();
    else await clientTransport.close();
    await Promise.all([rejected, backend.aborted]);
  });
}

test('factory validates absolute existing directory roots', () => {
  const backend = { call: async () => ({}) };
  assert.throws(() => createMcpServer({ root: '.' }, { backend }), /absolute/);
  assert.throws(() => createMcpServer({ root: entry }, { backend }), /directory/);
  assert.throws(() => createMcpServer({ root: join(root, 'missing-directory-for-mcp-test') }, { backend }));
});

test('CLI rejects invalid startup options on stderr only and import is safe', () => {
  const env = { ...process.env };
  delete env.SWE_GREP_ROOT;
  for (const args of [[], ['--root', '.'], ['--root', entry],
    ['--root', root, '--timeout-ms', '0'], ['--root', root, '--max-output-bytes', '1.2'],
    ['--root', root, '--unknown'], ['--root', root, 'positional']]) {
    const result = spawnSync(process.execPath, [entry, ...args], { env, encoding: 'utf8', timeout: 5000 });
    assert.equal(result.status, 1, result.stderr);
    assert.equal(result.stdout, '');
    assert.ok(result.stderr.length > 0);
  }
  const imported = spawnSync(process.execPath, ['--input-type=module', '-e',
    'await import(' + JSON.stringify(new URL('../src/mcp.js', import.meta.url).href) + ')'],
  { env, encoding: 'utf8', timeout: 5000 });
  assert.equal(imported.status, 0, imported.stderr);
  assert.equal(imported.stdout, '');
  assert.equal(imported.stderr, '');
});

test('official stdio client initializes and reports backend launch errors', { timeout: 10000 }, async t => {
  const transport = new StdioClientTransport({
    command: process.execPath, args: [entry, '--root', root, '--binary', join(root, 'no-such-swe-grep')],
    stderr: 'pipe',
  });
  let stderr = '';
  transport.stderr.on('data', chunk => { stderr += chunk; });
  const client = new Client({ name: 'stdio-test', version: '1.0.0' });
  t.after(async () => { await client.close(); });
  await client.connect(transport);
  assert.equal((await client.listTools()).tools.length, 3);
  const result = await client.callTool({ name: 'swe_search', arguments: search });
  assert.equal(result.isError, true);
  assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
  assert.equal(stderr, '');
});

for (const signal of ['SIGTERM', 'SIGINT']) {
  test('stdio accepts env root and shuts down gracefully on ' + signal, { timeout: 5000 }, async t => {
    const transport = new StdioClientTransport({ command: process.execPath,
      args: [entry, '--timeout-ms', '1000', '--max-output-bytes', '4096'],
      env: { SWE_GREP_ROOT: root }, stderr: 'pipe' });
    const client = new Client({ name: 'signal-test', version: '1.0.0' });
    t.after(() => client.close());
    let stderr = '';
    transport.stderr.on('data', chunk => { stderr += chunk; });
    await client.connect(transport);
    assert.equal((await client.listTools()).tools.length, 3);
    const closed = new Promise(resolve => { client.onclose = resolve; });
    process.kill(transport.pid, signal);
    await closed;
    assert.equal(transport.pid, null);
    assert.equal(stderr, '');
  });
}

test('direct executable guard supports npm-style symlink entries', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'swe-mcp-bin-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const link = join(directory, 'swe-grep-mcp');
  await symlink(entry, link);
  const result = spawnSync(process.execPath, [link, '--root', '.'], {
    env: process.env, encoding: 'utf8', timeout: 5000,
  });
  assert.equal(result.status, 1);
  assert.equal(result.stdout, '');
  assert.match(result.stderr, /absolute directory path/);
});

test('optional real backend fixture over official stdio transport', {
  skip: !process.env.SWE_GREP_BINARY, timeout: 60000,
}, async t => {
  const directory = await mkdtemp(join(tmpdir(), 'swe-mcp-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await writeFile(join(directory, 'fixture.js'), 'export function mcpFixtureSymbol() { return 1; }\n');
  const client = new Client({ name: 'real-test', version: '1.0.0' });
  const transport = new StdioClientTransport({ command: process.execPath,
    args: [entry, '--root', directory, '--binary', process.env.SWE_GREP_BINARY], stderr: 'pipe' });
  t.after(() => client.close());
  await client.connect(transport);
  const result = await client.callTool({ name: 'swe_search', arguments: { query: 'mcpFixtureSymbol' } });
  assert.notEqual(result.isError, true, JSON.stringify(result));
  assert.ok(result.structuredContent && typeof result.structuredContent === 'object');
  assert.match(JSON.stringify(result.structuredContent), /mcpFixtureSymbol/);
});
