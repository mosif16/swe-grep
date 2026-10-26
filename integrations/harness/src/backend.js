import { spawn } from 'node:child_process';
import { realpath, stat } from 'node:fs/promises';
import path from 'node:path';
import { validateInput } from './contracts.js';

function failure(code, message, details = {}) {
  return Object.assign(new Error(message), { code, ...details });
}

const localFilesystem = {
  async resolve(value, cwd, signal) {
    signal.throwIfAborted();
    const canonical = await realpath(path.resolve(cwd, value));
    const info = await stat(canonical);
    signal.throwIfAborted();
    return { path: canonical, kind: info.isDirectory() ? 'directory' : info.isFile() ? 'file' : 'other' };
  },
  contains(parent, child) {
    const relative = path.relative(parent.path, child.path);
    return relative === '' || (!path.isAbsolute(relative) && relative !== '..' && !relative.startsWith('..' + path.sep));
  },
};

// Keep launch essentials, not harness credentials, loader injection, or Git root overrides.
function localEnvironment(overrides) {
  const allowed = new Set([
    'PATH', 'PATHEXT', 'SYSTEMROOT', 'WINDIR', 'COMSPEC',
    'HOME', 'USERPROFILE', 'HOMEDRIVE', 'HOMEPATH',
    'TMPDIR', 'TMP', 'TEMP', 'LANG', 'LANGUAGE', 'LC_ALL', 'LC_CTYPE', 'TZ', 'XDG_CACHE_HOME',
  ]);
  return Object.assign(Object.fromEntries(Object.entries(process.env)
    .filter(([key]) => allowed.has(key.toUpperCase()))), overrides);
}

// This runner is deliberately local-only. Native hosts inject their policy-aware runner.
async function localRunner({ argv, cwd, env, signal, maxOutputBytes }) {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    let child;
    try {
      child = spawn(argv[0], argv.slice(1), { cwd, env: localEnvironment(env), shell: false, detached: process.platform !== 'win32', stdio: ['ignore', 'pipe', 'pipe'] });
    } catch (error) { reject(error); return; }
    const chunks = { stdout: [], stderr: [] };
    const sizes = { stdout: 0, stderr: 0 };
    const truncated = { stdout: false, stderr: false };
    let escalation;
    let spawnError;
    const kill = (name) => {
      try {
        if (process.platform !== 'win32' && child.pid) process.kill(-child.pid, name);
        else child.kill(name);
      } catch (error) { if (error.code !== 'ESRCH') spawnError ??= error; }
    };
    const stop = () => {
      if (escalation) return;
      kill('SIGTERM');
      escalation = setTimeout(() => kill('SIGKILL'), 100);
    };
    const collect = (stream, chunk) => {
      const cap = stream === 'stderr' ? Math.min(maxOutputBytes, 65536) : maxOutputBytes;
      const remaining = Math.max(0, cap - sizes[stream]);
      if (remaining) chunks[stream].push(chunk.subarray(0, remaining));
      sizes[stream] += chunk.length;
      if (sizes[stream] > cap) {
        truncated[stream] = true;
        if (stream === 'stdout') stop();
      }
    };
    child.stdout.on('data', chunk => collect('stdout', chunk));
    child.stderr.on('data', chunk => collect('stderr', chunk));
    child.on('error', error => { spawnError = error; });
    signal.addEventListener('abort', stop, { once: true });
    if (signal.aborted) stop();
    child.on('close', (exitCode, exitSignal) => {
      signal.removeEventListener('abort', stop);
      clearTimeout(escalation);
      // Reap remaining group members too, including children that closed their pipes.
      if (process.platform !== 'win32') kill('SIGKILL');
      if (spawnError) reject(spawnError);
      else resolve({ exitCode, signal: exitSignal, stdout: Buffer.concat(chunks.stdout).toString('utf8'), stderr: Buffer.concat(chunks.stderr).toString('utf8'), stdoutTruncated: truncated.stdout, stderrTruncated: truncated.stderr });
    });
  });
}

function interruptible(work, signal) {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const abort = () => {
      signal.removeEventListener('abort', abort);
      reject(signal.reason);
    };
    signal.addEventListener('abort', abort, { once: true });
    Promise.resolve().then(work).then(resolve, reject).finally(() => signal.removeEventListener('abort', abort));
  });
}

export class SweGrepBackend {
  constructor({ binary = 'swe-grep', root, runner = localRunner, filesystem = localFilesystem, timeoutMs = 30000, maxOutputBytes = 2097152 } = {}) {
    if (typeof binary !== 'string' || !binary || binary.includes('\0') || (!path.isAbsolute(binary) && /[/\\]/.test(binary))) {
      throw failure('SWE_GREP_INVALID_INPUT', 'binary must be a PATH command name or an absolute path');
    }
    if ((root !== undefined && (typeof root !== 'string' || !root || root.includes('\0'))) || typeof runner !== 'function' || typeof filesystem?.resolve !== 'function' || typeof filesystem?.contains !== 'function' || !Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > 2147483647 || !Number.isSafeInteger(maxOutputBytes) || maxOutputBytes <= 0) {
      throw failure('SWE_GREP_INVALID_INPUT', 'Invalid backend configuration');
    }
    Object.assign(this, { binary, root, runner, filesystem, timeoutMs, maxOutputBytes });
  }

  async call(toolName, args, { cwd = process.cwd(), signal } = {}) {
    const input = validateInput(toolName, args);
    const controller = new AbortController();
    const cancel = () => controller.abort(failure('SWE_GREP_ABORTED', 'swe-grep request aborted'));
    signal?.addEventListener('abort', cancel, { once: true });
    if (signal?.aborted) cancel();
    const timer = setTimeout(() => controller.abort(failure('SWE_GREP_TIMEOUT', 'swe-grep request timed out')), this.timeoutMs);
    const combined = controller.signal;
    try {
      const resolve = (value, base) => interruptible(() => this.filesystem.resolve(value, base, combined), combined);
      let rootInfo, target;
      try {
        rootInfo = await resolve(this.root ?? '.', cwd);
        target = await resolve(input.path ?? '.', rootInfo.path);
        if (rootInfo.kind !== 'directory' || !['directory', 'file'].includes(target.kind)) throw failure('SWE_GREP_INVALID_PATH', 'Expected a regular file or directory');
        if (!await interruptible(() => this.filesystem.contains(rootInfo, target), combined)) throw failure('SWE_GREP_PATH_OUTSIDE_ROOT', 'Path is outside the allowed root');
        if (toolName === 'swe_context' && target.kind !== 'directory') throw failure('SWE_GREP_INVALID_PATH', 'Context requires a directory; file context would widen scope');
        if (input.targetFile !== undefined) {
          const selectorBase = target.kind === 'directory' ? target.path : path.dirname(target.path);
          const selected = await resolve(input.targetFile, selectorBase);
          if (selected.kind !== 'file') throw failure('SWE_GREP_INVALID_PATH', 'targetFile must identify a regular file');
          if (!await interruptible(() => this.filesystem.contains(target, selected), combined)) throw failure('SWE_GREP_PATH_OUTSIDE_ROOT', 'targetFile is outside the requested analysis scope');
        }
      } catch (error) {
        if (combined.aborted) throw combined.reason;
        if (error.code?.startsWith('SWE_GREP_')) throw error;
        throw failure('SWE_GREP_INVALID_PATH', 'Cannot resolve requested path', { cause: error });
      }
      const command = { swe_search: 'search', swe_context: 'context', swe_refs: 'refs' }[toolName];
      const argv = [this.binary, '--no-config', '--disable-telemetry', command, '--symbol=' + input.query, '--path=' + target.path, '--quiet-hint'];
      if (toolName === 'swe_search') {
        argv.push('--json-only', '--concise', '--disable-fd', '--disable-ast-grep', '--top-hits=' + (input.limit ?? 20), '--max-matches=' + (input.limit ?? 20));
        if (input.mode === 'text') argv.push('--text');
        else argv.push('--mode=' + (input.mode ?? 'literal'));
        if (input.definitionsOnly) argv.push('--definitions-only');
        if (input.usagesOnly) argv.push('--usages-only');
        if (input.exactSymbol) argv.push('--exact-symbol');
      } else if (toolName === 'swe_context') {
        if (input.concise !== false) argv.push('--concise');
        if (input.direction === 'callers' || input.direction === 'both') argv.push('--callers');
        if (input.direction === 'callees' || input.direction === 'both') argv.push('--callees');
      } else {
        argv.push('--format=json', '--disable-fd', '--disable-ast-grep');
        if (input.direction !== 'callees') argv.push('--callers');
        if (input.direction !== 'callers') argv.push('--callees');
        if (input.excludeTests) argv.push('--exclude-tests');
        if (input.trackedOnly) argv.push('--tracked-only');
      }
      if (input.language) argv.push('--language=' + input.language);
      if (input.targetFile !== undefined) argv.push('--target-file=' + input.targetFile);
      if (input.targetLine !== undefined) argv.push('--target-line=' + input.targetLine);
      for (const value of input.exclude ?? []) argv.push('--exclude=' + value);
      for (const value of input.pathGlob ?? []) argv.push('--path-glob=' + value);
      const request = { argv, cwd: rootInfo.path, env: { SWE_GREP_DISABLE_TELEMETRY: '1', GIT_OPTIONAL_LOCKS: '0', GIT_CONFIG_COUNT: '1', GIT_CONFIG_KEY_0: 'core.fsmonitor', GIT_CONFIG_VALUE_0: 'false' }, signal: combined, maxOutputBytes: this.maxOutputBytes };
      let result;
      try {
        // Runners must honor signal and settle only after teardown. Never race their
        // cleanup against abort: native hosts own managed process lifetimes too.
        combined.throwIfAborted();
        result = await this.runner(request);
      } catch (error) {
        if (combined.aborted) throw combined.reason;
        throw failure(error.code === 'ENOENT' ? 'SWE_GREP_BINARY_NOT_FOUND' : 'SWE_GREP_EXECUTION_FAILED', 'Unable to execute swe-grep', { cause: error });
      }
      combined.throwIfAborted();
      if (!result || typeof result.stdout !== 'string' || typeof result.stderr !== 'string') throw failure('SWE_GREP_INVALID_OUTPUT', 'Runner returned an invalid result');
      if (result.stdoutTruncated || Buffer.byteLength(result.stdout) > this.maxOutputBytes) throw failure('SWE_GREP_OUTPUT_TRUNCATED', 'swe-grep output exceeded the output limit');
      const diagnostics = result.stderr.slice(0, 8192);
      let parsed;
      try { parsed = JSON.parse(result.stdout); } catch (cause) {
        throw failure(result.exitCode !== 0 || result.signal ? 'SWE_GREP_CLI_ERROR' : 'SWE_GREP_INVALID_OUTPUT', 'swe-grep did not return a complete JSON object', { exitCode: result.exitCode, diagnostics, cause });
      }
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw failure('SWE_GREP_INVALID_OUTPUT', 'swe-grep output must be a JSON object');
      if (result.exitCode !== 0 || result.signal || parsed.error || (Array.isArray(parsed.errors) && parsed.errors.length > 0)) throw failure('SWE_GREP_CLI_ERROR', 'swe-grep command failed', { exitCode: result.exitCode, diagnostics, output: parsed });
      return parsed;
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener('abort', cancel);
    }
  }
}
