/** Shared model-facing contracts; host adapters do not expose raw CLI arguments. */
const query = { type: 'string', description: 'Symbol, identifier, or search query. Maximum 256 UTF-8 bytes.' };
const path = { type: 'string', default: '.', description: 'Path within the configured workspace. Relative to the workspace root; cannot escape it.' };
const language = { type: 'string', description: 'Language hint such as rust, python, typescript, swift, or go.' };
const patterns = (description) => ({ type: 'array', items: { type: 'string' }, description });
const filters = {
  exclude: patterns('Exclude workspace-relative glob patterns.'),
  pathGlob: patterns('Limit results to workspace-relative glob patterns.'),
};
const schema = (properties) => ({ type: 'object', properties, required: ['query'], additionalProperties: false });
export const TOOLS = Object.freeze([
  {
    name: 'swe_search',
    description: 'Find relevant code locations with SWE-Grep. Returns bounded structured JSON; does not edit source files. Use swe_context for symbol understanding or swe_refs for semantic dependencies.',
    inputSchema: schema({ query, path, language, mode: { type: 'string', enum: ['literal', 'regex', 'text'], default: 'literal' }, limit: { type: 'integer', default: 20, description: 'Maximum returned matches, 1 through 100.' }, ...filters, definitionsOnly: { type: 'boolean', description: 'Only declaration sites matching the query.' }, usagesOnly: { type: 'boolean', description: 'Only non-declaration matches.' } }),
  },
  {
    name: 'swe_context',
    description: 'Understand a symbol: definition and callers/callees in a workspace directory. Compact structured JSON by default. Read-only with respect to source files.',
    inputSchema: schema({ query, path: { ...path, description: 'Workspace directory to analyze. Files are not accepted by this tool.' }, concise: { type: 'boolean', default: true } }),
  },
  {
    name: 'swe_refs',
    description: 'Find semantic callers/callees without guessing dependencies from unrelated code. Filters preserve graph analysis. Read-only with respect to source files.',
    inputSchema: schema({ query, path, language, direction: { type: 'string', enum: ['callers', 'callees', 'both'], default: 'both' }, ...filters, excludeTests: { type: 'boolean', default: false }, trackedOnly: { type: 'boolean', default: false } }),
  },
]);

function invalid(message) {
  const error = new Error(message);
  error.code = 'SWE_GREP_INVALID_INPUT';
  throw error;
}

/** Validate again at the backend boundary, not only in a particular host registry. */
export function validateInput(name, input) {
  const tool = TOOLS.find((tool) => tool.name === name);
  if (!tool) invalid('Unknown SWE-Grep tool: ' + String(name));
  if (!input || typeof input !== 'object' || Array.isArray(input) || ![Object.prototype, null].includes(Object.getPrototypeOf(input))) invalid('Tool arguments must be a plain object');
  for (const key of Object.keys(input)) {
    if (!Object.hasOwn(tool.inputSchema.properties, key)) invalid('Unknown argument: ' + key);
  }
  const args = {};
  for (const [key, spec] of Object.entries(tool.inputSchema.properties)) {
    const value = input[key] === undefined ? spec.default : input[key];
    if (value === undefined) continue;
    if (spec.type === 'string') {
      if (typeof value !== 'string' || value.includes(String.fromCharCode(0))) invalid(key + ' must be a string without NUL bytes');
      if (key === 'query' && (value.trim().length === 0 || Buffer.byteLength(value, 'utf8') > 256)) invalid('query must contain 1..256 UTF-8 bytes');
      if (key === 'path' && (value.length === 0 || Buffer.byteLength(value, 'utf8') > 4096)) invalid('path must contain 1..4096 UTF-8 bytes');
      if (key === 'language' && (!/^[a-zA-Z][a-zA-Z0-9_,+-]{0,63}$/.test(value))) invalid('language must be a language hint, not CLI arguments');
      if (spec.enum && !spec.enum.includes(value)) invalid(key + ' must be one of: ' + spec.enum.join(', '));
    } else if (spec.type === 'integer') {
      if (!Number.isSafeInteger(value) || value < 1 || value > 100) invalid(key + ' must be an integer in 1..100');
    } else if (spec.type === 'boolean') {
      if (typeof value !== 'boolean') invalid(key + ' must be boolean');
    } else if (spec.type === 'array') {
      if (!Array.isArray(value) || value.length > 20 || value.some((item) => typeof item !== 'string' || item.length === 0 || Buffer.byteLength(item, 'utf8') > 512 || item.includes(String.fromCharCode(0)))) invalid(key + ' must contain at most 20 nonempty glob strings, each at most 512 UTF-8 bytes');
    }
    args[key] = Array.isArray(value) ? value.slice() : value;
  }
  if (!Object.hasOwn(args, 'query')) invalid('query is required');
  if (args.definitionsOnly && args.usagesOnly) invalid('definitionsOnly and usagesOnly cannot be combined');
  return args;
}

export const validateToolCall = validateInput;
