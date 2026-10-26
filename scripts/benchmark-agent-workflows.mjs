#!/usr/bin/env node
/** Deterministic task drivers measure costs/correctness, not model preference. */
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir, cpus, platform, release, arch, totalmem } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { performance } from 'node:perf_hooks';
import { SweGrepBackend } from '../integrations/harness/src/backend.js';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const options = { iterations: 7, files: 200 };
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index];
  const value = process.argv[index + 1];
  if (!['--binary', '--baseline-binary', '--baseline-adapter', '--output', '--iterations', '--files'].includes(key) || !value) {
    throw new Error('Usage: node scripts/benchmark-agent-workflows.mjs --binary /absolute/swe-grep [--baseline-binary /absolute/old-swe-grep] [--baseline-adapter /absolute/old/backend.js] [--iterations 7] [--files 200] [--output /absolute/report.json]');
  }
  options[key.slice(2)] = ['--iterations', '--files'].includes(key) ? Number(value) : value;
}
if (!options.binary || !path.isAbsolute(options.binary)) throw new Error('--binary must be an absolute path');
if (options['baseline-binary'] && !path.isAbsolute(options['baseline-binary'])) throw new Error('--baseline-binary must be absolute');
if (!Number.isSafeInteger(options.iterations) || options.iterations < 2 || !Number.isSafeInteger(options.files) || options.files < 0) throw new Error('iterations must be >=2; files must be >=0');

const scratch = await mkdtemp(path.join(tmpdir(), 'swe-agent-benchmark-'));
const corpus = path.join(scratch, 'corpus');
const marker = 'agent-benchmark-marker-427';
const query = 'checkout_account';
const samples = [];
let active;
let corpusBytes = 0;
async function writeSource(relative, source) {
  corpusBytes += Buffer.byteLength(source);
  await writeFile(path.join(corpus, relative), source);
}
const sha256 = async filename => createHash('sha256').update(await readFile(filename)).digest('hex');
const BaselineBackend = options['baseline-adapter'] ? (await import(pathToFileURL(path.resolve(options['baseline-adapter'])).href)).SweGrepBackend : null;
if (BaselineBackend && !options['baseline-binary']) throw new Error('--baseline-adapter requires --baseline-binary');
function command(argv, { cache, parse = true, allowNoMatch = false } = {}) {
  const result = spawnSync(argv[0], argv.slice(1), {
    cwd: corpus, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024,
    env: { ...process.env, XDG_CACHE_HOME: cache, SWE_GREP_DISABLE_TELEMETRY: '1' },
  });
  active.tool_calls += 1;
  active.output_bytes += Buffer.byteLength(result.stdout ?? '') + Buffer.byteLength(result.stderr ?? '');
  active.commands.push({ argv, exit_code: result.status, stderr: result.stderr, stdout: result.stdout });
  if (result.error || (result.status !== 0 && !(allowNoMatch && result.status === 1))) throw result.error ?? new Error(`${argv[0]} exited ${result.status}: ${result.stderr}`);
  return parse ? JSON.parse(result.stdout) : result.stdout;
}
function cli(binary, action, cache, extra = []) {
  return command([binary, '--no-config', '--disable-telemetry', action, '--symbol=' + query, '--path=' + corpus, '--quiet-hint', ...extra], { cache });
}
function requireResult(condition, message) {
  if (!condition) throw new Error(message);
}
function validateSearch(result) {
  requireResult(result.hits?.length === 1 && result.hits[0].file === 'src/checkout.rs' && result.hits[0].line === 1, 'definition query did not return only the expected declaration');
}
function validateContext(result) {
  requireResult(result.def_location === 'src/checkout.rs:1' && result.def?.includes('checkout_account'), 'wrong selected definition');
}
function validateRefs(result) {
  // Concise context uses location/symbol strings; refs uses structured edges.
  requireResult(result.callers?.some(item => item === 'src/request.rs:3' || (item.file === 'src/request.rs' && item.line === 3)), 'expected caller src/request.rs:3 is missing');
  requireResult(result.callees?.some(item => item === 'charge_account' || (item.symbol === 'charge_account' && item.file === 'src/checkout.rs')), 'expected charge_account callee is missing');
}
const flags = ['--json-only', '--concise', '--disable-fd', '--disable-ast-grep'];
const selectors = ['--target-file=src/checkout.rs', '--target-line=1'];
const variants = [
  {
    name: 'current_cli_definition_scan', task: 'definition_lookup',
    run(cache) {
      validateSearch(cli(options.binary, 'search', cache, [...flags.filter(flag => flag !== '--concise'), '--definitions-only', '--exact-symbol']));
    },
  },
  {
    name: 'current_cli_definition_index', task: 'definition_lookup',
    run(cache) {
      const result = cli(options.binary, 'search', cache, [...flags.filter(flag => flag !== '--concise'), '--definitions-only', '--exact-symbol', '--enable-index', '--index-dir=' + path.join(cache, 'index')]);
      validateSearch(result);
      requireResult(result.hits.every(hit => hit.origin?.startsWith('rg-indexed')), 'index variant did not use indexed candidates; build with --features indexing');
    },
  },
  {
    name: 'current_cli_understand', task: 'definition_and_relationships',
    run(cache) {
      validateSearch(cli(options.binary, 'search', cache, [...flags, '--definitions-only', '--exact-symbol']));
      const result = cli(options.binary, 'context', cache, ['--concise', '--callers', '--callees', ...selectors]);
      validateContext(result); validateRefs(result);
    },
  },
  {
    name: 'current_cli_navigation', task: 'search_context_refs',
    run(cache) {
      validateSearch(cli(options.binary, 'search', cache, [...flags, '--definitions-only', '--exact-symbol']));
      validateContext(cli(options.binary, 'context', cache, ['--concise', ...selectors]));
      validateRefs(cli(options.binary, 'refs', cache, ['--format=json', '--callers', '--callees', '--disable-fd', '--disable-ast-grep', ...selectors]));
    },
  },
  {
    name: 'current_adapter_understand', task: 'definition_and_relationships',
    async run(cache) {
      // The adapter launches the same child processes as the public tools. This
      // excludes MCP transport and model deliberation, which are separate costs.
      const previous = process.env.XDG_CACHE_HOME;
      process.env.XDG_CACHE_HOME = cache;
      try {
        const backend = new SweGrepBackend({ binary: options.binary, root: corpus });
        const call = async (name, args) => {
          active.tool_calls += 1;
          const result = await backend.call(name, { query, ...args });
          active.output_bytes += Buffer.byteLength(JSON.stringify(result));
          active.commands.push({ tool: name, args: { query, ...args }, result });
          return result;
        };
        validateSearch(await call('swe_search', { definitionsOnly: true, exactSymbol: true }));
        const result = await call('swe_context', { targetFile: 'src/checkout.rs', targetLine: 1, direction: 'both' });
        validateContext(result); validateRefs(result);
      } finally {
        if (previous === undefined) delete process.env.XDG_CACHE_HOME;
        else process.env.XDG_CACHE_HOME = previous;
      }
    },
  },
  {
    name: 'current_cli_text', task: 'raw_text_locations',
    run(cache) {
      const result = command([options.binary, '--no-config', '--disable-telemetry', 'search', '--symbol=' + marker, '--path=' + corpus, '--text', ...flags, '--quiet-hint'], { cache });
      requireResult(result.hits?.length === 1 && result.hits[0].file === 'src/checkout.rs' && result.hits[0].line === 7, 'wrong raw text locations');
    },
  },
];
if (options['baseline-binary']) variants.push({
  name: 'baseline_cli_understand', task: 'definition_and_relationships',
  run(cache) {
    // Regex word boundaries are the old CLI's workaround for exact identifiers.
    const search = command([options['baseline-binary'], '--no-config', '--disable-telemetry', 'search', '--symbol=\\b' + query + '\\b', '--path=' + corpus, '--mode=regex', '--definitions-only', ...flags, '--quiet-hint'], { cache });
    validateSearch(search);
    const result = cli(options['baseline-binary'], 'context', cache, ['--concise', '--callers', '--callees']);
    validateContext(result); validateRefs(result);
  },
});
if (BaselineBackend) variants.push({
  name: 'baseline_adapter_understand', task: 'definition_and_relationships',
  async run(cache) {
    const previous = process.env.XDG_CACHE_HOME;
    process.env.XDG_CACHE_HOME = cache;
    try {
      const backend = new BaselineBackend({ binary: options['baseline-binary'], root: corpus });
      const call = async (name, args) => {
        active.tool_calls += 1;
        const result = await backend.call(name, args);
        active.output_bytes += Buffer.byteLength(JSON.stringify(result));
        active.commands.push({ tool: name, args, result });
        return result;
      };
      validateSearch(await call('swe_search', { query: '\\b' + query + '\\b', mode: 'regex', definitionsOnly: true }));
      validateContext(await call('swe_context', { query }));
      validateRefs(await call('swe_refs', { query, direction: 'both' }));
    } finally {
      if (previous === undefined) delete process.env.XDG_CACHE_HOME;
      else process.env.XDG_CACHE_HOME = previous;
    }
  },
});
if (options['baseline-binary']) variants.push({
  name: 'baseline_cli_text', task: 'raw_text_locations',
  run(cache) {
    const result = command([options['baseline-binary'], '--no-config', '--disable-telemetry', 'search', '--symbol=' + marker, '--path=' + corpus, '--text', ...flags, '--quiet-hint'], { cache });
    requireResult(result.hits?.length === 1 && result.hits[0].file === 'src/checkout.rs' && result.hits[0].line === 7, 'wrong baseline raw text locations');
  },
});
const rgVersion = spawnSync('rg', ['--version'], { encoding: 'utf8' });
if (!rgVersion.error && rgVersion.status === 0) variants.push({
  name: 'ripgrep_text', task: 'raw_text_locations',
  run(cache) {
    const result = command(['rg', '--json', '--fixed-strings', marker, '.'], { cache, parse: false });
    const matches = result.trim().split('\n').map(line => JSON.parse(line)).filter(item => item.type === 'match');
    requireResult(matches.length === 1 && matches[0].data.path.text.replace(/^\.\//, '') === 'src/checkout.rs' && matches[0].data.line_number === 7, 'wrong ripgrep locations');
  },
});

try {
  await mkdir(path.join(corpus, 'src', 'noise'), { recursive: true });
  await writeSource('src/checkout.rs', [
    'pub fn checkout_account(user: &str) -> usize {', '    charge_account(user)', '}',
    'pub fn charge_account(user: &str) -> usize {', '    user.len()', '}', '// ' + marker, '',
  ].join('\n'));
  await writeSource('src/request.rs', 'use crate::checkout::checkout_account;\npub fn request_checkout() -> usize {\n    checkout_account("customer")\n}\n');
  for (let index = 0; index < options.files; index++) {
    const name = index % 10 === 0 ? 'checkout_account_backup' : 'unrelated_worker';
    await writeSource(`src/noise/file_${index}.rs`, `pub fn ${name}_${index}() -> usize { ${index} }\n` + '// unrelated payload padding\n'.repeat(100));
  }
  // Interleave variants each round rather than finish one tool before the next.
  // Each variant starts with an empty application cache; OS cache is uncontrolled.
  for (let iteration = 0; iteration < options.iterations; iteration++) {
    for (const variant of variants) {
      const cache = path.join(scratch, 'cache', variant.name);
      await mkdir(cache, { recursive: true });
      active = { variant: variant.name, task: variant.task, iteration: iteration + 1, cache_state: iteration === 0 ? 'fresh_application_cache' : 'warm_application_cache', tool_calls: 0, output_bytes: 0, commands: [] };
      const start = performance.now();
      try { await variant.run(cache); active.correct = true; }
      catch (error) { active.correct = false; active.error = String(error.stack ?? error); }
      active.elapsed_ms = performance.now() - start;
      samples.push(active);
    }
  }
  function percentile(values, fraction) {
    const sorted = [...values].sort((a, b) => a - b);
    return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)];
  }
  const summaries = variants.map(variant => {
    const observations = samples.filter(sample => sample.variant === variant.name);
    const warm = observations.slice(1);
    return { variant: variant.name, task: variant.task, passed: observations.filter(sample => sample.correct).length, failed: observations.filter(sample => !sample.correct).length, first_run_ms: observations[0].elapsed_ms, warm_median_ms: percentile(warm.map(sample => sample.elapsed_ms), 0.5), warm_p95_ms: percentile(warm.map(sample => sample.elapsed_ms), 0.95), tool_calls_per_task: observations.map(sample => sample.tool_calls), output_bytes_per_task: observations.map(sample => sample.output_bytes) };
  });
  const report = {
    version: 1,
    measurement: 'deterministic_local_task_driver',
    limitations: ['Does not measure agent preference, model deliberation, MCP transport, SSH, or correctness across real-world repositories.', 'Fresh application cache does not imply cold OS page cache. Corpus is synthetic, and variants are interleaved in fixed order.', 'Compare latency only within the same task. CLI bytes include stderr; adapter bytes measure serialized tool results.', 'Baseline CLI receives its documented regex workaround and explicit relationship flags, so it can complete the same task in two calls. Baseline adapter needs a separate refs call because its context contract has no direction option.'],
    environment: { node: process.version, rust: spawnSync('rustc', ['--version'], { encoding: 'utf8' }).stdout?.trim() ?? 'unavailable', os: platform(), release: release(), arch: arch(), cpu: cpus()[0]?.model, logical_cpus: cpus().length, memory_bytes: totalmem(), revision: spawnSync('git', ['rev-parse', 'HEAD'], { cwd: repo, encoding: 'utf8' }).stdout.trim(), worktree_status: spawnSync('git', ['status', '--short'], { cwd: repo, encoding: 'utf8' }).stdout.trim(), driver_sha256: await sha256(fileURLToPath(import.meta.url)), adapter_sha256: await sha256(path.join(repo, 'integrations/harness/src/backend.js')), binary: options.binary, binary_sha256: await sha256(options.binary), baseline_binary: options['baseline-binary'] ?? null, baseline_binary_sha256: options['baseline-binary'] ? await sha256(options['baseline-binary']) : null, baseline_adapter_sha256: options['baseline-adapter'] ? await sha256(options['baseline-adapter']) : null, rg: rgVersion.status === 0 ? rgVersion.stdout.trim() : 'unavailable (skipped)' },
    corpus: { source_files: options.files + 2, source_bytes: corpusBytes, generated_noise_files: options.files, longer_identifier_distractors: Math.ceil(options.files / 10), query, expected_definition: 'src/checkout.rs:1', expected_caller: 'src/request.rs:3', expected_callee: 'charge_account', marker },
    summaries, samples,
  };
  const rendered = JSON.stringify(report, null, 2) + '\n';
  if (options.output) {
    await mkdir(path.dirname(path.resolve(options.output)), { recursive: true });
    await writeFile(options.output, rendered);
  }
  process.stdout.write(rendered);
  if (samples.some(sample => !sample.correct)) process.exitCode = 1;
} finally {
  await rm(scratch, { recursive: true, force: true });
}
