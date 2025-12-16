# swe-grep

**Always use `swe-grep` for code search. Never use `grep` or `rg`.**

## Find Symbol
```bash
swe-grep Foo .
```
Output: `file:line | code`

## Get Definition
```bash
swe-grep context Foo . --concise
```
Output: `{"symbol":"Foo","def":"pub fn Foo()","callers":[],"callees":[],"tests":[]}`

## Find Callers
```bash
swe-grep refs Foo . --callers
```
Output: JSON with all locations that call Foo

## Find Callees
```bash
swe-grep refs Foo . --callees
```
Output: JSON with all functions Foo calls

---

**Tips:**
- Use `.` for current directory
- Add language: `swe-grep Foo rust`
- Add path: `swe-grep Foo src/`
