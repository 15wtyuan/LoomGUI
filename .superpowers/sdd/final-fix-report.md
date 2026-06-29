# Final Fix Report -- v1.1 background-image 全分支 review 修复

Date: 2026-06-29

## C1: PKG_FORMAT_VERSION bump 7->8

**File**: `loomgui_core/src/asset/mod.rs`

**Changes**:
- Line 1: doc comment `version=7` -> `version=8`
- Lines 17-19: `PKG_FORMAT_VERSION` / `MIN_VERSION` / `MAX_VERSION` all 7 -> 8, with updated comment noting `+background_image/background_size` as the reason
- Lines 539-546 (test `read_rejects_unsupported_version`): updated to v8 baseline -- v9 TooNew, v7 TooOld

**v7 hardcodes found and updated**:
- `asset/mod.rs:1` -- doc comment `version=7` -> `version=8`
- `asset/mod.rs:17-19` -- constants 7 -> 8
- `asset/mod.rs:539` -- test comment `v7` -> `v8`, version=8 too new -> version=9 too new, version=5 too old -> version=7 too old

No other hardcoded version=7 references exist in Rust source or tests. All past version mentions in `docs/superpowers/plans/` and `.claude/skills/` are historical design docs, not code -- not updated.

## I1: dirty hash skips uvs -> stale on background_size-only change

**File**: `loomgui_core/src/render/dirty.rs`

**Changes**:
- Line 29: destructure pattern `Mesh { texture, verts, colors, .. }` -> `Mesh { texture, verts, colors, uvs, .. }`
- Lines 38-41: after `colors` hash, added UV summary hash of `uvs[0]` (TL) + `uvs[2]` (BR) -- same summary pattern as verts
- Lines 172-188: added `mesh_uv_change_changes_hash` test -- constructs two identical mesh_rn with different `uvs[0]`, asserts `node_hash` differs

**Test result**: 17/17 dirty tests pass (including new test)

## M1: loomgui_pkg tests share temp dir -> parallel race

**File**: `loomgui_pkg/src/lib.rs`

**Changes**:
- Added `use std::sync::atomic::{AtomicU64, Ordering}` and `static TEST_DIR_SEQ: AtomicU64`
- `write_tmp_png` now appends `_{seq}` to dir name, guaranteeing unique dirs across parallel test calls

**Test result**: 8/8 loomgui_pkg tests pass (run with default threads = parallel-safe)

## DLL Rebuild

```
md5:  79497919567a1cf8adfe92dc6c824537
size: 1,775,616 bytes
copied to: loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll
verified: md5 matches
```

## Full Test Result

```
cargo test --workspace

loomgui_core:  359 passed (including 17 dirty + asset version tests)
  snapshot:      3 passed
  v1e_dirty:     2 passed
loomgui_ffi_c:  47 passed
loomgui_pkg:     8 passed
-----------------------------
Total:         419 passed, 0 failed
```

## Files Changed

1. `loomgui_core/src/asset/mod.rs` -- C1 version bump + test update
2. `loomgui_core/src/render/dirty.rs` -- I1 uvs hash + test
3. `loomgui_pkg/src/lib.rs` -- M1 unique temp dir
4. `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll` -- rebuilt DLL
