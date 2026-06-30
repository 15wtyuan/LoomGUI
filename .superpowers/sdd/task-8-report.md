## Task 8 Report: render/mod.rs Container/Image 分流 + dirty hash

### What was implemented

1. **render/mod.rs Container branch (lines 126-186)**: Expanded tuple from `(texture, u_min, u_max)` to `(texture, u_min, u_max, src_w, src_h)` with `m.width as f32`/`m.height as f32` cast from `TexMeta`'s `u32` fields. Replaced `if all_zero` 2-mode with `match (has_slice, all_zero)` 4-mode routing to quad/rounded_rect/nine_slice/nine_slice_rounded. Added `has_filter` check: `color_filter.is_some() → program=3`, else `texture!=0 → 2`, else `0`. Added `color_matrix = n.style.color_filter.unwrap_or([0.0; 20])`.

2. **render/mod.rs Image branch (lines 187-207)**: Expanded tuple to include `src_w, src_h`. Added `match &n.style.border_image_slice` routing to quad/nine_slice (no radius -- YAGNI). Added same `has_filter → program=3` and `color_matrix` logic.

3. **render/dirty.rs node_hash (lines 29-34)**: Changed `NodePayload::Mesh { texture, verts, colors, uvs, .. }` to `{ texture, verts, colors, uvs, program, color_matrix, .. }`. Added `program.hash(&mut h)` and `for &v in color_matrix.iter() { v.to_le_bytes().hash(&mut h); }` before existing verts.len() hash.

### TDD Evidence (RED → GREEN)

**RED phase:**
- `build_container_with_filter_sets_program_3`: FAILED -- program=0 (expected 3)
- `build_container_with_slice_uses_nine_slice`: FAILED -- verts.len()=4, quad instead of 16-vert nine_slice
- `build_container_no_filter_keeps_program_0_or_2`: PASS (zero-regression, already correct)

**GREEN phase:** All 3 tests pass after implementation.

### Files Changed

- `loomgui_core/src/render/mod.rs`: Container 4-mode mesh dispatch + program=3 logic + color_matrix; Image 2-mode slice support + filter
- `loomgui_core/src/render/dirty.rs`: node_hash Mesh arm now captures program + color_matrix

### Self-Review Findings

- **Completeness**: Container four-mode (quad/rounded_rect/nine_slice/nine_slice_rounded) via `(has_slice, all_zero)` match. Image two-mode (quad/nine_slice) only, no radius (YAGNI). program=3 for both when filter. color_matrix via unwrap_or zeros.
- **Quality**: program logic correct -- has_filter → 3; else bg-image hit → 2; else 0 (Container) / else 0 (Image). color_matrix zeros when no filter (program won't be 3, MatrixManager won't read it).
- **Discipline**: No Image+radius. No new mesh functions. Dirty hash changes minimal -- only program+color_matrix added before existing verts/uvs logic, all existing hash logic preserved unchanged.
- **u32→f32 cast**: `m.width as f32, m.height as f32` at the tuple destructure site -- correct, clean, not threaded through fit_uv.
- **Regression safety**: All 477 workspace tests pass (414 core + 55 ffi_c + 8 pkg). Existing bg-image, program=0/2, rounded_rect, border-radius, UV flip, merge, dirty hash, and scrollbar tests all green.

### Concerns

None.
