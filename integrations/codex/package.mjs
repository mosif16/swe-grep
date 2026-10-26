#!/usr/bin/env node
/** Build a new, self-contained marketplace. Never modify an existing install. */
import { cpSync, mkdirSync, copyFileSync, readFileSync, writeFileSync, chmodSync, realpathSync, accessSync, constants } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const here = fileURLToPath(new URL('.', import.meta.url));
const writeJSON = (file, value) => writeFileSync(file, JSON.stringify(value, null, 2) + '\n');
const shellQuote = value => "'" + value.replaceAll("'", "'\\''") + "'";

export function packagePlugin({ binary, routes, destination }) {
  binary = realpathSync(binary);
  accessSync(binary, constants.X_OK);
  if (!routes || typeof routes.localNode !== 'string' || !routes.localNode ||
      typeof routes.identity?.account !== 'string' || !Array.isArray(routes.identity?.hostnames) ||
      !routes.identity.hostnames.length || routes.identity.hostnames.some(name => typeof name !== 'string' || !name)) {
    throw new Error('Routes require localNode, identity.account and identity.hostnames');
  }
  // Exclusive creation also rejects empty existing directories and symlinks.
  const output = resolve(destination);
  mkdirSync(output);
  const plugin = join(output, 'plugins', 'swe-grep');
  mkdirSync(join(plugin, 'bin'), { recursive: true });
  const adapter = join(plugin, 'adapter');
  mkdirSync(adapter);
  for (const name of ['package.json', 'package-lock.json', 'LICENSE']) {
    copyFileSync(join(here, '../harness', name), join(adapter, name));
  }
  cpSync(join(here, '../harness/src'), join(adapter, 'src'), { recursive: true });
  copyFileSync(join(here, 'server.mjs'), join(plugin, 'server.mjs'));
  cpSync(join(here, 'skills'), join(plugin, 'skills'), { recursive: true });
  copyFileSync(join(adapter, 'LICENSE'), join(plugin, 'LICENSE'));
  const binaryName = process.platform === 'win32' ? 'swe-grep.exe' : 'swe-grep';
  copyFileSync(binary, join(plugin, 'bin', binaryName));
  chmodSync(join(plugin, 'bin', binaryName), 0o755);
  // Resolve the binary relative to the installed plugin, including after cache copies.
  writeJSON(join(plugin, 'routes.json'), { ...routes, binary: './bin/' + binaryName });
  const version = '0.2.2';
  writeJSON(join(plugin, 'plugin.json'), {
    $schema: 'https://agent-plugins.org/schemas/1.0.0/plugin.schema.json',
    name: 'swe-grep', version, description: 'SWE-Grep search, definitions and references on explicitly selected nodes.',
    author: { name: 'Record and Learn LLC' }, license: 'SEE LICENSE IN LICENSE',
    extensions: { 'com.openai': { interface: { displayName: 'SWE-Grep', shortDescription: 'Search code and trace symbols with SWE-Grep' } } },
  });
  mkdirSync(join(plugin, '.codex-plugin'));
  writeJSON(join(plugin, '.codex-plugin/plugin.json'), {
    name: 'swe-grep', version, description: 'SWE-Grep code navigation', skills: './skills/', mcpServers: './.mcp.json',
  });
  // The desktop loader requires the portable command to remain inside the plugin.
  const launcher = process.platform === 'win32' ? './start-mcp.cmd' : './start-mcp.sh';
  writeJSON(join(plugin, 'mcp.json'), {
    $schema: 'https://agent-plugins.org/schemas/1.0.0/mcp.schema.json',
    mcpServers: { 'swe-grep': { type: 'stdio', command: launcher } },
  });
  writeJSON(join(plugin, '.mcp.json'), { mcpServers: { 'swe-grep': {
    command: process.execPath, args: ['${CLAUDE_PLUGIN_ROOT}/server.mjs'],
  } } });
  writeFileSync(join(plugin, 'start-mcp.sh'), '#!/bin/sh\nexec ' + shellQuote(process.execPath) + ' "$(dirname "$0")/server.mjs" "$@"\n');
  chmodSync(join(plugin, 'start-mcp.sh'), 0o755);
  writeFileSync(join(plugin, 'start-mcp.cmd'), '@echo off\r\n"' + process.execPath + '" "%~dp0server.mjs" %*\r\n');
  mkdirSync(join(output, '.agents/plugins'), { recursive: true });
  writeJSON(join(output, '.agents/plugins/marketplace.json'), {
    name: 'recordandlearn-local', interface: { displayName: 'Record and Learn Local' },
    plugins: [{ name: 'swe-grep', source: { source: 'local', path: './plugins/swe-grep' },
      policy: { installation: 'AVAILABLE', authentication: 'ON_INSTALL' }, category: 'Productivity' }],
  });
  const npmArgs = ['ci', '--omit=dev', '--omit=optional', '--ignore-scripts', '--workspaces=false'];
  const npmCli = process.platform === 'win32' ? join(process.execPath, '..', 'node_modules/npm/bin/npm-cli.js') : null;
  const result = spawnSync(npmCli ? process.execPath : 'npm', npmCli ? [npmCli, ...npmArgs] : npmArgs,
    { cwd: adapter, stdio: 'inherit', timeout: 120000 });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error('Dependency installation failed; partial output retained at ' + output);
  return plugin;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [binary, routesFile, destination, ...extra] = process.argv.slice(2);
    if (!binary || !routesFile || !destination || extra.length) throw new Error('Usage: node package.mjs MATCHING_BINARY ROUTES_JSON NEW_MARKETPLACE_DIRECTORY');
    console.log(packagePlugin({ binary, routes: JSON.parse(readFileSync(routesFile, 'utf8')), destination }));
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
