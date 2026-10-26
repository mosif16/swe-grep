#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';
import { serve, executeTool, verifyIdentity } from './adapter/src/codex.js';

try {
  const { values } = parseArgs({ options: {
    config: { type: 'string' }, 'local-only': { type: 'boolean' },
    tool: { type: 'string' }, args: { type: 'string' },
  }, strict: true });
  const config = JSON.parse(readFileSync(values.config ?? fileURLToPath(new URL('./routes.json', import.meta.url)), 'utf8'));
  config.binary = resolve(dirname(values.config ? resolve(values.config) : fileURLToPath(new URL('./routes.json', import.meta.url))), config.binary);
  if (values['local-only']) config.routes = {};
  verifyIdentity(config);
  if (values.tool) {
    const args = JSON.parse(values.args ?? readFileSync(0, 'utf8'));
    args.node ??= config.localNode;
    process.stdout.write(JSON.stringify(await executeTool(config, values.tool, args)) + '\n');
  } else await serve(config);
} catch (error) {
  process.stderr.write('swe-grep: ' + error.message + '\n');
  process.exitCode = 1;
}
