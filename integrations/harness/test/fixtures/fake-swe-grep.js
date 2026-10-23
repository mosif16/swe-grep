#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
const query = process.argv.find(value => value.startsWith('--symbol='))?.slice(9);
if (query === 'hang' || query === 'tree') {
  process.on('SIGTERM', () => {});
  let child;
  if (query === 'tree') child = spawn(process.execPath, ['-e', 'process.on("SIGTERM", () => {}); setInterval(() => {}, 1000)'], { stdio: 'inherit' });
  writeFileSync('processes.json', JSON.stringify({ parent: process.pid, child: child?.pid }));
  setInterval(() => {}, 1000);
} else if (query === 'flood') {
  process.stdout.write('x'.repeat(1000000));
  setInterval(() => {}, 1000);
} else if (query === 'diagnostics') {
  process.stderr.write('d'.repeat(1000000));
  process.stdout.write('{"ok":true}');
} else if (query === 'invalid') {
  process.stdout.write('{');
} else if (query === 'error') {
  process.stdout.write('{"error":{"message":"failed"}}');
  process.exitCode = 2;
} else {
  process.stdout.write(JSON.stringify({ argv: process.argv.slice(2), cwd: process.cwd() }));
}
