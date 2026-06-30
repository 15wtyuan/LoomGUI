# Final Fix Report — C1 (Critical) + I1 (Important)

Date: 2026-06-30

## C1: mergeRuleFile regex collapse `[\\s\\S]` → `[^]`

**Root cause**: In template literal `` `${BEGIN}[\\s\\S]*?${END}` ``, the `\\s` and `\\S` collapse to literal `s` and `S` (one backslash consumed by template literal). The resulting regex `/<!-- loomgui-editor-begin -->[sS]*?<!-- loomgui-editor-end -->/g` only matches literal `s`/`S` characters, never matches the tagged block. `existing.replace(re, ...)` returns `existing` unchanged, so old rules are silently retained while `action` reports `"updated"`.

**Fix**: Changed `[\\s\\S]*?` to `[^]*?` (any char including newlines, no escaping issues in template literals).

**File**: `editor/init.mjs` line 45.

## C1 Test Gap: unit test for mergeRuleFile

**Problem**: Task 4 manual test was a false negative — it only checked tag count and user content preservation, not whether old content was actually replaced.

**Fix**: Created `editor/init.test.mjs` with 3 tests using `node:test`:

| Test | Assertion |
|---|---|
| `created` | No file → `"created"`, content written |
| `appended` | File without tags → `"appended"`, user content preserved + rules added |
| `updated` | File with v1 tags → `"updated"`, **v2 present AND v1 absent** (would fail pre-fix) |

**Also**: Added ESM entry guard `if (process.argv[1] && resolve(process.argv[1]) === __filename)` so `main()` doesn't run on import.

**Result**: `node --test editor/init.test.mjs` → 3/3 pass, 0 fail.

## I1: fence.md copy sync

**Problem**: Task 7 modified `docs/design/fence.md` (14标注 changes) but didn't sync to `editor/skill/loomgui-editor/references/fence.md`.

**Fix**: `cp docs/design/fence.md editor/skill/loomgui-editor/references/fence.md`

**Verification**: `diff -q` → no output (byte-identical).

## Verification Commands

```
node --test editor/init.test.mjs   # 3 pass, 0 fail
diff -q docs/design/fence.md editor/skill/loomgui-editor/references/fence.md  # no output
```
