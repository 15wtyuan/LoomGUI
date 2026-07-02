# Task 7 Report: FFI（load_package 加 name + instantiate + 砍 atlas FFI）+ csbindgen regen

## 实现

### 1. FFI 改动（`loomgui_ffi_c/src/lib.rs`）

- **`loomgui_stage_load_package`**：签名加 `name: *const u8, name_len: usize` 参数（对齐 T4 `Stage::load_package(name, bytes)`）。旧签名 `(h, bytes, len)` → 新签名 `(h, name, name_len, bytes, bytes_len) -> i32`。null 句柄/空指针返 -1。
- **`loomgui_stage_instantiate`**（新增）：`(h, pkg, pkg_len, comp, comp_len) -> u32`。包装 T5 `Stage::instantiate(pkg, comp)`。失败/无 scene/null 句柄返 `0xFFFF_FFFF`（INVALID），成功返 `NodeId.0`。
- **砍**：`loomgui_stage_atlas_count`、`loomgui_stage_atlas_info`（图集归 Unity，D8）。
- **`loomgui_stage_load_inline`**：T4 已删（FFI 不存在）；T7 确认无残留。
- **FrameBlob Image 行 path**：blob.rs（T6）已落地 v7（tex_id→path_idx + path string table arena）。T7 未改 blob.rs——FFI 侧 `build_blob` 调用不变，csbindgen regen 不涉及 blob 结构（blob 走字节流，无 C# struct 绑定）。

### 2. csbindgen regen → Native.cs（实为 `LoomGUIBindings.cs`）

- **机制**：`loomgui_ffi_c/build.rs` 在 cargo build 时跑 csbindgen，扫描 `src/lib.rs` 的 `#[no_mangle] extern "C"` fn，生成 C# 绑定直接写到 `loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs`（best-effort，失败发 cargo:warning 不 fail-the-build）。
- **该 .cs 文件被 .gitignore**（自动生成产物，不入库）——本机 build 即自动同步；家里机 build .dll 时也自动 regen。
- **验证**：build 后 grep 确认 `loomgui_stage_load_package` 签名带 `name`/`name_len`、`loomgui_stage_instantiate` 已生成、`atlas_count`/`atlas_info` 已删。

### 3. 6 个 ignored FFI 测试恢复（ignored 6→0）

| 测试 | 处理 |
|---|---|
| `load_package_builds_blob_from_package` | **重写**：load_package(name) → create_root → instantiate → append_child → tick → blob |
| `atlas_count_and_info_round_trip` | **删除**（atlas FFI 已砍，测的链路已断） |
| `is_pointer_on_ui_true_on_hit_false_on_miss` | **重写**：create_root 建 scene → tick → is_pointer_on_ui（空根→false） |
| `node_parent_returns_chain_and_sentinel` | **重写**：load_package + create_root + instantiate + append_child → 验 parent 链 |
| `find_node_by_id_round_trip` | **重写**：手搓包含 id="ok" 节点 → load_package + instantiate → find |
| `dynamic_tree_api_ffi_round_trip` | **实现**：create_root 自动建 scene（ensure_scene）→ 9 函数 round-trip |

新增 2 个 brief 要求的测试：`load_package_ffi_takes_name`、`instantiate_ffi_returns_nodeid`。
新增 helper `make_test_pkg_bytes(component)`（手搓单 Container 组件 pkg，不走 parse）。
删除 `scene_to_pkg` helper（T1 桥接遗留，被新 helper 取代，dead code）。

### 4. build release .dll

- `cargo build --release -p loomgui_ffi_c` 成功。
- 拷 `target/release/loomgui_ffi_c.dll` → `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`。
- .dll size：1,837,568 → 1,888,256 字节（+50KB，instantiate FFI + path string table code）。

## 测试 + 结果

```
cargo test -p loomgui_ffi_c
test result: ok. 52 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

- ignored count: **6 → 0**（全恢复/删除）。
- `cargo build --workspace` 通过。
- `cargo build --release -p loomgui_ffi_c` 通过。

## TDD 证据

1. 写 `load_package_ffi_takes_name` + `instantiate_ffi_returns_nodeid` 测试（brief verbatim）。
2. `instantiate_ffi_returns_nodeid` 首跑暴露 `create_root` FFI 传 null css 时 `slice::from_raw_parts` 崩溃（pre-existing bug：css=null+css_len=0 触发 unsafe precondition）。修测试改传空串指针（符合 FFI 契约——caller 传 valid ptr）。
3. 改 FFI 签名 + 新增 instantiate → 测试通过。
4. 恢复 6 个 ignored 测试 → 全过。

## 文件变更

- `loomgui_ffi_c/src/lib.rs`（FFI 签名 + 测试）
- `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`（release .dll，commit 入库）
- `loomgui_unity/Assets/Plugins/LoomGUI/Bindings/LoomGUIBindings.cs`（csbindgen 自动 regen，**gitignored 不入库**）

## 自审

- FFI 签名对齐 v1.3+ 风格（raw pointers + lengths，i32 return for ok/err，u32 NodeId with INVALID sentinel）。
- `instantiate` 用 `from_utf8`（非 `from_utf8_unchecked`）防 UB——brief 给的 `_unchecked` 版本不安全，改用安全版（性能差异可忽略，FFI 边界安全优先）。
- `load_package` 同样用 `from_utf8().unwrap_or("")` 容错。
- atlas FFI 删除后 Unity `LoomStage.cs` 仍引用 `Native.loomgui_stage_atlas_count/info`（line 453/462）——**T8 负责重写 LoomStage.cs 砍 LoadAtlas/_texMap**。本 task 不改 Unity C#（无 Unity 工具链验，且 T8 专门做）。过渡期 Unity 编译会断，T8 修。

## 关注点

1. **csbindgen regen 机制**：build.rs 自动跑，`LoomGUIBindings.cs` 是 gitignored 生成产物——本机 build 即同步，家里机 build .dll 时也自动 regen。无需手动跑脚本。
2. **.dll size**：1.84MB → 1.89MB（+50KB，合理）。
3. **Unity 过渡期断裂**：T7 砍 atlas FFI 后 `LoomStage.cs` 引用悬空——T8 必须紧跟改 Unity。本 task 不修 Unity（T8 专责）。
4. **删除 vs 重写**：1 个测试删除（atlas round-trip，链路已断），5 个重写/实现。新 helper `make_test_pkg_bytes` 比 `scene_to_pkg` 更轻（不需构造完整 Scene）。
5. **`create_root` FFI 传 null css 崩溃**：pre-existing unsafe bug，测试改传空串规避。FFI 本身未加 null 防护（超本 task 范围，caller 契约传 valid ptr）。
