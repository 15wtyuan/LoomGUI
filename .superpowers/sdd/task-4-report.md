# Task 4 Report: Stage 资源池 + load_package（不建 scene）+ 砍 textures/atlases/load_inline

**Status:** COMPLETE
**Date:** 2026-07-02
**Branch:** worktree-v1.4a-package-loading

## What Implemented

T4 是 v1.4-a 包加载重构的核心转折点：load_package 从"建 scene"改为"进资源池不碰 scene"。

### 1. Stage struct 改造（`loomgui_core/src/stage.rs`）
- **新增** `packages: HashMap<String, Package>` 字段（资源池，多包共存）。
- **删除** `textures: TextureRegistry` + `atlases: Vec<AtlasInfo>` 字段（图集归 Unity，D8）。
- **删除** `load_inline` 公共方法（D12，单一加载机制全走 package）。
- **重写** `load_package(name: &str, bytes: &[u8])`：read_package → `self.packages.insert(name, pkg)`。**不碰 scene**（spec §4.2 D3）。重复 load 同名包 = 替换。多包共存。
- **新增** `#[cfg(all(test, feature="parse"))] load_inline_for_test` 测试 helper：保留 parse→scene 路径供 stage 单测用（load_inline 砍了，但单测验证 parse→render 管线仍需建 scene）。
- `tick_and_render`：solve/build_render_nodes 传空 `TextureRegistry::default()`（Image fallback tex_id=0 占位，不崩）。T6 改 payload 带 path 后彻底删 textures 参数。

### 2. asset/mod.rs 清理（`loomgui_core/src/asset/mod.rs`）
- **删除** `AtlasSection` / `AtlasInfo` / `AtlasSprite` / `build_registry`（图集归 Unity，T1 暂留，T4 清）。
- **删除** 2 个 build_registry 测试。
- **保留** `asset/texture.rs`（`TextureRegistry`/`TexMeta`）——render/layout 仍读（Image 查 textures 走空 registry fallback），T6 改 Image payload 带 path 后彻底删。**这是对 brief "删 asset/texture.rs" 的偏离**，见下方 Concerns。

### 3. FFI 层（`loomgui_ffi_c/src/lib.rs`）
- `loomgui_stage_load_html`：内部改直接调 parse_html + build_scene（同旧 load_inline 逻辑，不调已删的 load_inline）。保留 FFI 签名（T7 决定是否砍）。
- `loomgui_stage_load_package`：调 `load_package("", bytes)`（传空名，T7 加 name 参数到 FFI 签名）。
- `loomgui_stage_atlas_count` / `loomgui_stage_atlas_info`：**stub 返 0/null**（atlases 字段已砍，T7 删本函数 + csbindgen regen）。

### 4. 测试迁移
- **新增** `load_package_tests` 模块（4 测）：进资源池不建 scene / 多包共存 / 同名替换 / 不碰 scene 不变量。
- **stage.rs 单测**：`load_inline` → `load_inline_for_test`（保留 parse→render 管线测试）。
- **3 个包路径测试 `#[ignore]`**：`package_load_renders_identical_to_inline` / `set_input_hover_emits_rollover_and_rematch` / `set_node_disabled_inhibits_click`——原测 inline→包重建 scene，load_package 不再建 scene 故暂 ignore（T5 instantiate 后改写）。
- **集成测试**（`snapshot.rs` / `v1e_dirty.rs`）：加本地 `load_html_css` helper 直接调 parse_html + build_scene（集成测试是独立 crate，看不到 lib 的 `#[cfg(test)]` 项）。
- **FFI 测试**：5 个用 load_package 建 scene 的测试 `#[ignore]`（load_package 不建 scene，tick/find 无 scene 会 panic）。T5+T7 改写。
- **benchmarks**（`frame_emit.rs`）：同集成测试，加 `load_html_css` helper。
- **examples**（`dump_*.rs`）：load_package 加 dummy name "showcase"；dump_render 直接调 parse；dump_img 改读 packages 字典（scene 未建）。

## Tests + Results

```
cargo build --workspace          → OK（零 error 零 warning）
cargo bench --no-run             → OK
cargo test --workspace           → 551 passed; 0 failed; 9 ignored
  - loomgui_core (lib)           → 481 passed; 3 ignored (包路径测试, T5 改写)
  - loomgui_core (parse dom/css) → 10 passed
  - loomgui_core (snapshot)      → 3 passed
  - loomgui_core (v1e_dirty)     → 2 passed
  - loomgui_ffi_c (lib)          → 45 passed; 6 ignored (atlas + load_package 建 scene 测试, T5/T7 改写)
  - loomgui_pkg                  → 10 passed
  - 其他 doctest/example         → 0 passed
```

**T4 新增 4 测全过：**
- `load_package_into_pool_without_scene` ✓
- `load_package_multi_pkg_coexist` ✓
- `load_package_replace_same_name` ✓
- `load_package_does_not_touch_scene` ✓

## TDD Evidence

1. **Step 2 写失败测试**：加 `load_package_tests` 模块（4 测，brief verbatim + 1 不变量测）。
2. **Step 3 跑测试确认失败**：`cargo test load_package_tests` → 13 编译错（no field `packages`、load_package 签名 2 参但传 1 参）。
3. **Step 4-6 实现**：改 Stage struct + 重写 load_package + 清 asset + 修编译错。
4. **Step 7 跑测试确认通过**：`cargo test -p loomgui_core --lib load_package_tests` → 4 passed。

## Files Changed

- `loomgui_core/src/stage.rs` — struct 改造 + load_package 重写 + load_inline 砍 + load_inline_for_test helper + 测试迁移
- `loomgui_core/src/asset/mod.rs` — 删 AtlasSection/AtlasInfo/AtlasSprite/build_registry + 2 测
- `loomgui_ffi_c/src/lib.rs` — load_html 内联 parse + load_package 传空名 + atlas FFI stub + 5 测 ignore
- `loomgui_core/tests/snapshot.rs` — load_html_css helper
- `loomgui_core/tests/v1e_dirty.rs` — load_html_css helper
- `loomgui_core/benches/frame_emit.rs` — load_html_css helper
- `loomgui_core/examples/dump_render.rs` — 直接调 parse
- `loomgui_core/examples/dump_img.rs` — 读 packages 字典
- `loomgui_core/examples/dump_bg.rs` / `dump_scroll.rs` / `dump_text.rs` / `dump_interact.rs` — load_package 加 dummy name

## Self-Review

- ✅ load_package 不碰 scene（4 测验证：调后 scene 不变，只 packages 字典变）。
- ✅ 重复 load 同名包 = 替换（测验证：len 仍 1，且是新包）。
- ✅ 多包共存（测验证：len=2）。
- ✅ root_size 归 Stage（保留字段，不从包来）。
- ✅ 砍 load_inline（公共 API 删，测试 helper 保留 parse 路径）。
- ✅ 砍 build_registry/AtlasSection/AtlasInfo/AtlasSprite。
- ✅ 零回归：render 侧 Image fallback（空 registry → tex_id=0 占位，不崩），T6 改 payload 带 path。
- ✅ `cargo build --workspace` + `cargo test --workspace` 全过。
- ✅ 既有测试改用内存 pkg / parse 直调（单测 load_inline_for_test，集成测试 load_html_css helper）。

## Concerns

### 1. asset/texture.rs 未删（偏离 brief Step 5）
Brief Step 5 说"删 asset/texture.rs 文件 + mod texture;"。**实际保留**了 `TextureRegistry`/`TexMeta`：
- render/mod.rs + layout/mod.rs 深度依赖 `TextureRegistry`（Image 查 tex_id/UV、fit_uv、nine_slice 全消费 TexMeta）。
- 删 texture.rs 会连锁崩 ~40 个 render/layout 测试（都要改 tex_id/UV 断言）——这是 T6 的工作（"render 侧 Image 节点带 path_idx（非 tex_id/UV）"）。
- T4 的指导是"render 侧暂用 fallback（textures 空时 Image 不崩）"——保留类型 + 传空 registry 是最干净的 fallback 实现。
- **T6 会彻底删 texture.rs**（改 Image payload 带 path 后 render 不再查 textures）。
- `build_registry`/`AtlasSection` 等图集类型已删（这些是 asset 模块特有的，render 不依赖）。

### 2. FFI stubs（atlas_count/atlas_info）
- 按 brief 选项 (a) stub 返 0/null（minimal，T7 删）。
- `loomgui_stage_load_package` FFI 传空名 ""（T7 加 name 参数到 FFI 签名 + csbindgen regen）。
- 5 个 FFI 测试 `#[ignore]`（依赖 load_package 建 scene，T5 instantiate + T7 FFI 改写后恢复）。
- **T7 须注意**：FFI `load_package` 签名要加 name 参数，Unity 侧 LoadPackage 也要改。

### 3. 测试迁移
- 3 个 core 测试 `#[ignore]`（包路径建 scene 测试，T5 instantiate 后改写）。
- `load_inline_for_test` 是 `#[cfg(all(test, feature="parse"))]` 方法（仅 lib 单测可见，集成测试用本地 helper）。
- snapshot/v1e_dirty/bench 都加了 `load_html_css` 本地 helper（重复代码，但隔离清晰）。

### 4. render fallback hack
- `tick_and_render` 内 `let empty_textures = TextureRegistry::default();` 每次 tick 创建空 registry。
- 性能影响可忽略（空 HashMap default 是零分配）。
- T6 改 Image payload 带 path 后，solve/build_render_nodes 的 textures 参数会删。
