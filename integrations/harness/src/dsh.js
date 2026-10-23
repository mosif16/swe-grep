import Schema from '@deepseek-ai/schemastery';
import { defineTool, assertObjectJsonSchema, validateJsonSchemaValue, ToolArgsError } from '@deepseek-ai/dsh-tools';
import { TOOLS } from './contracts.js';
import { SweGrepBackend } from './backend.js';

export const name = 'swe-grep';
export const inject = ['tools', 'workingDirectory', 'subprocess', 'fs'];
export const Config = Schema.object({
  binary: Schema.string().min(1).description('Execution-world executable name or absolute path; defaults to swe-grep.'),
  root: Schema.string().min(1).description('Optional search boundary, resolved relative to the calling session directory.'),
  timeoutMs: Schema.natural().min(1).max(2147483647).default(30000),
  maxOutputBytes: Schema.natural().min(1).max(Number.MAX_SAFE_INTEGER).default(2097152),
});

// JSON Schema required arrays become per-property required:true in the SDK DSL.
function properties(schema) {
  return Object.fromEntries(Object.entries(schema.properties ?? {}).map(([key, value]) => [key, {
    ...valueSpec(value),
    ...(schema.required?.includes(key) ? { required: true } : {}),
  }]));
}
function valueSpec(schema) {
  const { required, properties: _properties, additionalProperties, items, oneOf, ...value } = schema;
  if (oneOf) return { ...value, oneOf: oneOf.map(valueSpec) };
  if (schema.type === 'object') return { ...value, properties: properties(schema), additionalProperties: additionalProperties ?? true };
  if (schema.type === 'array') return { ...value, ...(items ? { items: valueSpec(items) } : {}) };
  return Object.keys(value).length ? value : { type: 'json' };
}

export function apply(ctx, config = {}) {
  const options = Config(config);
  // Keep canonical provider identities opaque, even when paths look host-local.
  const targetKey = Symbol('swe-grep filesystem target');
  const filesystem = {
    async resolve(path, cwd, signal) {
      const target = await ctx.fs.resolve(path, { cwd, signal });
      const info = await ctx.fs.stat(target, signal);
      if (!info || !['file', 'directory'].includes(info.type)) {
        throw new Error('SWE-Grep requires an existing regular file or directory');
      }
      return { path: ctx.fs.processPath(target), kind: info.type, [targetKey]: target };
    },
    contains(root, path) {
      return ctx.fs.contains(root[targetKey], path[targetKey]);
    },
  };
  const runner = async ({ argv, cwd, env, signal, maxOutputBytes }) => {
    // Resolution must run inside the backend deadline and the execution world.
    const executable = await ctx.subprocess.resolveExecutable(argv[0], env, signal);
    const child = ctx.subprocess.spawn({
      argv: [executable, ...argv.slice(1)], cwd, env, signal,
      stdio: { stdin: 'ignore', stdout: { maxBytes: maxOutputBytes }, stderr: { maxBytes: maxOutputBytes } },
      graceMs: 1000,
    });
    let outcome;
    try {
      outcome = await child.done;
    } finally {
      // Even rejection/cancellation must not abandon managed descendants.
      if (!await child.waitForExit()) {
        throw new Error('SWE-Grep managed process range did not exit');
      }
    }
    const stdout = child.collected.stdout.readFrom(0);
    const stderr = child.collected.stderr.readFrom(0);
    return { ...outcome, stdout: stdout.text, stderr: stderr.text,
      stdoutTruncated: stdout.lossy, stderrTruncated: stderr.lossy };
  };
  const backend = new SweGrepBackend({ ...options, binary: options.binary ?? 'swe-grep', runner, filesystem });
  for (const tool of TOOLS) {
    assertObjectJsonSchema(tool.inputSchema);
    const definition = defineTool({
      name: tool.name,
      description: tool.description,
      parameters: properties(tool.inputSchema),
      output: {
        schema: { type: 'object', additionalProperties: true },
        render: (_args, value) => [{ type: 'text', text: JSON.stringify(value) }],
      },
      // No timeoutMs or concurrency opt-in: backend owns the deadline and
      // exclusive dispatch avoids concurrent CLI cache/index writers.
      async execute(args, exec) {
        const violations = validateJsonSchemaValue(tool.inputSchema, args, '');
        if (violations.length) throw new ToolArgsError(violations);
        if (exec.agent === undefined) throw new Error('SWE-Grep requires an agent Session');
        const cwd = await ctx.workingDirectory.ensure(exec.agent, exec.signal);
        return backend.call(tool.name, args, { cwd, signal: exec.signal });
      },
    });
    // The DSL parameter root is implicitly open. Preserve the canonical contract
    // (including additionalProperties:false) in registration and validation.
    ctx.tools.register({ ...definition, parameters: tool.inputSchema });
  }
}
