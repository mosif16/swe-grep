import test from 'node:test';
import assert from 'node:assert/strict';
import os from 'node:os';
import path from 'node:path';
import { mkdtempSync, writeFileSync, readFileSync, rmSync, cpSync, realpathSync } from 'node:fs';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { packagePlugin } from '../../codex/package.mjs';
import { executeTool } from '../src/codex.js';

const binary = process.env.SWE_GREP_BINARY;
const identity = { account: os.userInfo().username, hostnames: [os.hostname()] };

test('packaged plugin survives cache relocation and verifies routed node identity', {
  skip: !binary, timeout: 60000,
}, async t => {
  const temp = mkdtempSync(path.join(os.tmpdir(), 'swe plugin package '));
  t.after(() => rmSync(temp, { recursive: true, force: true }));
  const destination = path.join(temp, 'marketplace');
  const original = packagePlugin({ binary, destination, routes: { localNode: 'fixture-node', identity, routes: {} } });
  assert.throws(() => packagePlugin({ binary, destination, routes: { localNode: 'fixture-node', identity } }), { code: 'EEXIST' });
  const manifest = JSON.parse(readFileSync(path.join(original, 'plugin.json')));
  assert.equal(manifest.version, '0.2.2');
  assert.match(readFileSync(path.join(original, 'skills/swe-grep/SKILL.md'), 'utf8'), /swe_refs/);
  const portable = JSON.parse(readFileSync(path.join(original, 'mcp.json')));
  assert.equal(portable.mcpServers['swe-grep'].command, process.platform === 'win32' ? './start-mcp.cmd' : './start-mcp.sh');
  const marketplace = JSON.parse(readFileSync(path.join(destination, '.agents/plugins/marketplace.json')));
  assert.equal(marketplace.plugins[0].source.path, './plugins/swe-grep');
  const relocated = path.join(temp, 'cache copy');
  cpSync(original, relocated, { recursive: true });
  rmSync(destination, { recursive: true });
  const file = path.join(temp, 'fixture.py');
  writeFileSync(file, 'def package_target():\n    return 1\n\ndef caller():\n    return package_target()\n');
  const client = new Client({ name: 'package-test', version: '1.0.0' });
  const transport = new StdioClientTransport({
    command: path.join(relocated, process.platform === 'win32' ? 'start-mcp.cmd' : 'start-mcp.sh'),
    stderr: 'pipe',
  });
  t.after(() => client.close());
  await client.connect(transport);
  const listed = await client.listTools();
  assert.equal(listed.tools.length, 3);
  const args = { node: 'fixture-node', query: 'package_target', path: temp };
  for (const name of ['swe_search', 'swe_context', 'swe_refs']) {
    const result = await client.callTool({ name, arguments: args });
    assert.notEqual(result.isError, true, JSON.stringify(result));
    assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
    assert.equal(result.structuredContent._swe.node, 'fixture-node');
    assert.equal(result.structuredContent._swe.path, realpathSync(temp));
    if (name === 'swe_search') assert.ok(result.structuredContent.hits.some(x=>x.file==='fixture.py' && x.line===1));
    if (name === 'swe_context') assert.equal(result.structuredContent.def_location, 'fixture.py:1');
    if (name === 'swe_refs') assert.ok(result.structuredContent.callers.some(x=>x.file==='fixture.py' && x.line===5));
  }
  const remoteConfig = {
    localNode: 'router-node', identity, binary,
    routes: { 'fixture-node': { command: process.execPath, args: [path.join(relocated, 'server.mjs')] } },
  };
  const routed = await executeTool(remoteConfig, 'swe_search', args);
  assert.equal(routed._swe.node, 'fixture-node');
  // A reachable server claiming another node must fail, never fall back locally.
  remoteConfig.routes.wrong = remoteConfig.routes['fixture-node'];
  await assert.rejects(executeTool(remoteConfig, 'swe_search', { ...args, node: 'wrong' }),
    { code: 'SWE_GREP_NODE_UNAVAILABLE' });
  // Also simulate a server misidentifying an otherwise successful response.
  const fakeServer = path.join(temp, 'wrong-server.mjs');
  const sdk = new URL('../node_modules/@modelcontextprotocol/sdk/dist/esm/', import.meta.url).href;
  writeFileSync(fakeServer, `
    import {Server} from ${JSON.stringify(sdk+'server/index.js')};
    import {StdioServerTransport} from ${JSON.stringify(sdk+'server/stdio.js')};
    import {CallToolRequestSchema} from ${JSON.stringify(sdk+'types.js')};
    const server=new Server({name:'wrong',version:'1'},{capabilities:{tools:{}}});
    server.setRequestHandler(CallToolRequestSchema,async()=>({content:[],structuredContent:{_swe:{node:'wrong'}}}));
    await server.connect(new StdioServerTransport());
  `);
  remoteConfig.routes['fixture-node'] = { command: process.execPath, args: [fakeServer] };
  await assert.rejects(executeTool(remoteConfig, 'swe_search', args), { code:'SWE_GREP_NODE_IDENTITY_MISMATCH' });
});
