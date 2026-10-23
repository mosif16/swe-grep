#!/usr/bin/env node
import { realpathSync, statSync } from 'node:fs';
import { isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { CallToolRequestSchema, ListToolsRequestSchema, McpError, ErrorCode } from '@modelcontextprotocol/sdk/types.js';
import { TOOLS, validateToolCall } from './contracts.js';
import { SweGrepBackend } from './backend.js';

const annotations = Object.freeze({
  readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false,
});

function validateRoot(root) {
  if (typeof root !== 'string' || !isAbsolute(root)) {
    throw new Error('--root (or SWE_GREP_ROOT) must be an absolute directory path');
  }
  if (!statSync(root).isDirectory()) throw new Error('--root must be a directory');
}

/** Create an unconnected SDK server. Arguments cannot control backend configuration. */
export function createMcpServer(options, { backend } = {}) {
  validateRoot(options.root);
  backend ??= new SweGrepBackend(options);
  const lifecycle = new AbortController();
  const server = new Server({ name: 'swe-grep', version: '0.1.1' }, {
    capabilities: { tools: { listChanged: false } },
  });
  const close = server.close.bind(server);
  server.close = async () => {
    lifecycle.abort(new Error('MCP server closed'));
    await close();
  };
  server.onclose = () => lifecycle.abort(new Error('MCP transport disconnected'));
  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: TOOLS.map(tool => ({ ...tool, annotations })),
  }));
  server.setRequestHandler(CallToolRequestSchema, async (request, extra) => {
    const { name, arguments: args = {} } = request.params;
    let validated;
    try {
      validated = validateToolCall(name, args);
    } catch (error) {
      throw new McpError(ErrorCode.InvalidParams, error.message);
    }
    try {
      const result = await backend.call(name, validated, {
        signal: AbortSignal.any([extra.signal, lifecycle.signal]),
      });
      return { structuredContent: result, content: [{ type: 'text', text: JSON.stringify(result) }] };
    } catch (error) {
      const result = { error: {
        code: typeof error?.code === 'string' ? error.code : 'BACKEND_ERROR',
        message: error instanceof Error ? error.message : String(error),
      } };
      return { isError: true, structuredContent: result,
        content: [{ type: 'text', text: JSON.stringify(result) }] };
    }
  });
  return server;
}

function positiveInteger(value, flag) {
  if (value === undefined) return undefined;
  if (!/^[1-9]\d*$/.test(value) || !Number.isSafeInteger(Number(value))) {
    throw new Error(flag + ' must be a positive safe integer');
  }
  return Number(value);
}

async function main() {
  const { values } = parseArgs({ options: {
    root: { type: 'string' }, binary: { type: 'string' },
    'timeout-ms': { type: 'string' }, 'max-output-bytes': { type: 'string' },
  }, strict: true, allowPositionals: false });
  const server = createMcpServer({
    root: values.root ?? process.env.SWE_GREP_ROOT,
    binary: values.binary,
    timeoutMs: positiveInteger(values['timeout-ms'], '--timeout-ms'),
    maxOutputBytes: positiveInteger(values['max-output-bytes'], '--max-output-bytes'),
  });
  const shutdown = () => { void server.close().catch(error => {
    process.stderr.write('swe-grep MCP: ' + error.message + '\n');
    process.exitCode = 1;
  }); };
  process.once('SIGTERM', shutdown);
  process.once('SIGINT', shutdown);
  // SDK stdio does not close itself when stdin reaches EOF.
  process.stdin.once('end', shutdown);
  process.stdin.once('close', shutdown);
  process.stdout.once('error', shutdown);
  const onclose = server.onclose;
  server.onclose = () => {
    onclose();
    process.removeListener('SIGTERM', shutdown);
    process.removeListener('SIGINT', shutdown);
    process.stdin.removeListener('end', shutdown);
    process.stdin.removeListener('close', shutdown);
    process.stdout.removeListener('error', shutdown);
  };
  await server.connect(new StdioServerTransport());
}

function isDirectEntry() {
  if (!process.argv[1]) return false;
  try { return realpathSync(process.argv[1]) === fileURLToPath(import.meta.url); }
  catch { return false; }
}

if (isDirectEntry()) {
  main().catch(error => {
    process.stderr.write('swe-grep MCP: ' + error.message + '\n');
    process.exitCode = 1;
  });
}
