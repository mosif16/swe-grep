import test from 'node:test';
import assert from 'node:assert/strict';
import { TOOLS, validateInput } from '../src/contracts.js';

test('all hosts share three strict read-only query contracts', () => {
  assert.deepEqual(TOOLS.map(tool => tool.name), ['swe_search', 'swe_context', 'swe_refs']);
  for (const tool of TOOLS) {
    assert.equal(tool.inputSchema.additionalProperties, false);
    assert.deepEqual(tool.inputSchema.required, ['query']);
    assert.equal(validateInput(tool.name, {query: 'Target'}).path, '.');
  }
  assert.equal(validateInput('swe_search', {query: 'Target'}).limit, 20);
  assert.equal(validateInput('swe_refs', {query: 'Target'}).direction, 'both');
  assert.equal(validateInput('swe_context', {query: 'Target'}).concise, true);
  assert.equal(validateInput('swe_context', {query: 'Target'}).direction, undefined);
  assert.equal(validateInput('swe_context', {query: 'Target', language: 'rust', targetFile: 'src/lib.rs', targetLine: 230}).targetLine, 230);
});

test('invalid or ambiguous requests never reach a process', () => {
  for (const [name, args] of [
    ['unknown', {query: 'Target'}], ['swe_search', {}],
    ['swe_search', {query: ' '}], ['swe_search', {query: 'é'.repeat(129)}],
    ['swe_search', {query: 'ok', binary: 'other'}],
    ['swe_search', {query: 'ok', root: '/'}],
    ['swe_search', {query: 'ok', limit: 101}],
    ['swe_search', {query: 'ok', limit: 1.5}],
    ['swe_search', {query: 'ok', definitionsOnly: true, usagesOnly: true}],
    ['swe_search', {query: 'ok', exclude: ['x'.repeat(513)]}],
    ['swe_search', {query: 'ok', pathGlob: Array(21).fill('*.rs')}],
    ['swe_search', {query: 'ok', exactSymbol: true, mode: 'regex'}],
    ['swe_search', {query: 'ok', exactSymbol: true, mode: 'text'}],
    ['swe_context', {query: 'ok', language: '--rust'}],
    ['swe_context', {query: 'ok', targetLine: 1}],
    ['swe_context', {query: 'ok', targetFile: 'src/lib.rs', targetLine: 0}],
    ['swe_context', {query: 'ok', targetFile: 'src/lib.rs', targetLine: 1.5}],
    ['swe_context', {query: 'ok', targetFile: 'src/lib.rs', targetLine: 2147483648}],
    ['swe_refs', {query: 'ok', direction: 'delete'}],
    ['swe_refs', {query: 'ok', trackedOnly: 'true'}],
  ]) {
    assert.throws(() => validateInput(name, args), {code: 'SWE_GREP_INVALID_INPUT'});
  }
});

test('definition selectors reject absolute paths and parent traversal before filesystem access', () => {
  for (const name of ['swe_context', 'swe_refs']) {
    for (const targetFile of ['', ' ', '/etc/passwd', '../outside.py', 'src/../../outside.py', 'C:\\outside.py', '\\\\server\\share\\code.py', 'src\\..\\outside.py', 'x'.repeat(4097), 'src/\0code.py']) {
      assert.throws(() => validateInput(name, {query: 'target', targetFile}), {code: 'SWE_GREP_INVALID_INPUT'});
    }
    assert.equal(validateInput(name, {query: 'target', targetFile: 'src/a file.py', targetLine: 2147483647}).targetFile, 'src/a file.py');
  }
});

test('validation copies mutable arrays and permits option-looking query data', () => {
  const input = {query: '--clear-cache', exclude: ['*.lock']};
  const args = validateInput('swe_search', input);
  args.exclude.push('*.md');
  assert.equal(input.exclude.length, 1);
  assert.equal(args.query, '--clear-cache');
});
