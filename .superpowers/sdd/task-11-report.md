## Task 11 Report: set_scroll_pos FFI + v1d.5 + .dll

**Status**: DONE
**Commit**: `5609854` feat(v1d.5-T11): set_scroll_pos FFI + version v1d.5 + .dll rebuild
**Date**: 2026-06-26

### Changes

| File | Change |
|------|--------|
| `loomgui_core/src/stage.rs` | +14 lines: `set_scroll_pos(node, x, y, animated)` method |
| `loomgui_ffi_c/src/lib.rs` | +114/-5: FFI `loomgui_stage_set_scroll_pos` + version v1d.4→v1d.5 + 6 tests |
| `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll` | rebuilt (1,755,136 bytes, MD5 81661b4bc79d65c5b736972b6cfe11b4) |

### Diff Summary

1. **Stage::set_scroll_pos** (stage.rs:166-179): delegates to `scene.scroll.get_mut(node)` then `set_pos((x,y), animated)`. Non-scroll container / OOB node → no-op (no panic). Follows `set_wheel_input` pattern.

2. **FFI `loomgui_stage_set_scroll_pos`** (lib.rs:409-421): null-safe, `animated: u8` (0=snap 1=cubic-out tween), delegates to `stage.set_scroll_pos(NodeId(...), x, y, animated != 0)`.

3. **Version v1d.4 → v1d.5**: `CString::new("v1d.5")` + doc comment + test renamed `version_is_v1d_5`.

### Tests Added (all pass)

- `set_scroll_pos_updates_state` — snap scroll_pos to (0,50)
- `set_scroll_pos_animated_starts_tween` — animated=true sets tweening=1
- `set_scroll_pos_non_container_no_op` — non-scroll node → no panic
- `set_scroll_pos_oob_no_op` — NodeId(99) out of bounds → no panic
- `ffi_set_scroll_pos_round_trip` — FFI animated=0/1 round-trip via parse feature
- `version_is_v1d_5` — version string == "v1d.5"

### Test Results

`cargo test --workspace`: **356 passed, 0 failed**

| Crate | Tests |
|-------|-------|
| loomgui_core | 305 + 3 snapshots |
| loomgui_ffi_c | 45 |
| loomgui_pkg | 3 + 3 pack |

### .dll Rebuild

- `cargo build --release -p loomgui_ffi_c` → `target/release/loomgui_ffi_c.dll`
- Copied to `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`
- Size: 1,755,136 bytes (was 1,740,288)
- MD5: `81661b4bc79d65c5b736972b6cfe11b4`

### Self-Review

| Check | Result |
|-------|--------|
| set_scroll_pos delegates to T6 set_pos (not direct scroll_pos write) | PASS |
| Non-scroll container → scroll.get_mut returns None → no-op, no panic | PASS |
| OOB NodeId → node.0 < scene.nodes.len() guard → no-op, no panic | PASS |
| FFI null-safe (h.is_null() guard) | PASS |
| animated: u8 → animated != 0 conversion | PASS |
| Version string v1d.4 → v1d.5 (CString + doc + tests) | PASS |
| .dll committed (坑 10: two-machine constraint) | PASS |
| Workspace tests all green | PASS |

### Concerns

None. All tests green, .dll rebuilt and committed.
