<INSTRUCTIONS>
Use `swe-grep` for all code search in this repo (do not use `grep`/`rg`).

## 4 commands you need

### 1) Find occurrences
```bash
swe-grep Foo .
```
Output is one line per hit:
`path:line | snippet`

### 2) Get the definition (fast JSON)
```bash
swe-grep context Foo . --concise
```
Output:
`{"symbol":"Foo","def":"...","callers":[],"callees":[],"tests":[]}`

### 3) Find callers
```bash
swe-grep refs Foo . --callers
```
Output is JSON; look at `.callers[]` (`file`, `line`, `snippet`).

### 4) Find callees
```bash
swe-grep refs Foo . --callees
```
Output is JSON; look at `.callees[]` (`symbol`, `file`, `line`, `snippet`).

## Useful tricks

- Limit to one file: `swe-grep Foo path/to/file.rs`
- Language hint: `swe-grep Foo rust` or `swe-grep refs --language rust Foo . --callers`
- Machine output (search subcommand): `swe-grep search Foo . --json-only`
</INSTRUCTIONS>
