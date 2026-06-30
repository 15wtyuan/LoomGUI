# v1.1 background-image 共存视觉修复（坑 79）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Container 设 background-image 时，图透明区显 background-color（CSS background 合成），兑现 v1.1 §6.2 承诺。

**Architecture:** 用 program 号区分 img（0，tex×vcol）与 Container+bg-image（2，CSS 合成）。program 字段已在 `NodePayload::Mesh`/`Text` 就位，`MaterialManager` 已按 program keying——核心 gap 仅 FFI 序列化层：blob 没序列化 program，MirrorPool 硬编码 0。加 program 列（u8，第 19 列）进 frame blob，shader 加 `BG_COMPOSITE` keyword 走合成公式。

**Tech Stack:** Rust（loomgui_core / loomgui_ffi_c）+ Unity 6.5 URP（C# / HLSL）。

## Global Constraints

- **program 列类型 u8**（与 payload_kind/visible 同类，1 字节，值域 0/1/2）。spec §3.3。
- **blob VERSION 4→5**。FrameBlob.cs `ExpectedVersion` 同步 4→5。spec §3.3。
- **program 号语义**：0=img/无图 Container（`tex*vcol`）；1=Text（ALPHA_MASK）；2=Container+bg-image（CSS 合成）。spec §3.1。
- **合成公式**：`col.rgb=tex.rgb*tex.a+vcol.rgb*(1-tex.a); col.a=vcol.a;`（`#if defined(BG_COMPOSITE)`）。spec §3.2。
- **Container program 取值**（mod.rs）：有 `background_image` 且 `textures.get(url).is_some()` → program=2；否则 program=0。spec §3.1。
- **pkg.bin 零改**：program 是 frame blob 字段，不是 scene/ResolvedStyle 字段，PKG_FORMAT_VERSION 不动（仍 v9）。spec §5。
- **img 节点 / text 路径零改**：img 保持 program=0，text 保持 program=1。spec §5。
- **圆角+bg-image 共存安全**：rounded_rect 镂空区无 fragment，合成不需特判。spec §3.4。
- **两台机串行**：本机唯一编码机（Rust build .dll + commit + push）；家里机纯 Unity PlayMode 验收。改 Rust 后必重编 .dll 家里机才能测。
- **main 直推**。

---

## File Structure

| 文件 | 责任 | 改动类型 |
|---|---|---|
| `loomgui_ffi_c/src/blob.rs` | frame blob 序列化（Rust） | 加 program 列 + VERSION 5 |
| `loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs` | frame blob 解析（C#） | ColOff(18) + Program(i) + ExpectedVersion 5 |
| `loomgui_core/src/render/mod.rs` | Scene → RenderNode | Container 分支 program=2 |
| `loomgui_unity/Assets/LoomGUI/Shaders/LoomGUI-Unlit.shader` | frag shader | BG_COMPOSITE 分支 |
| `loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs` | Material 缓存 | program==2 keyword |
| `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs` | 渲染分发 | 用 blob.Program(i) |
| `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll` | FFI 二进制 | 重编 |

---

## Task 1: blob.rs 加 program 列（u8）+ VERSION 4→5

**Files:**
- Modify: `loomgui_ffi_c/src/blob.rs`

**Interfaces:**
- Consumes: `RenderNode.payload` 的 `program` 字段（`NodePayload::Mesh { ..., program }` / `Text { ..., program }`，已存在）。
- Produces: blob 第 19 列 `program`（u8），TestView `col_off: [usize; 19]`，`view.program(i)` 读 u8。

**现状关键**：Mesh arm `NodePayload::Mesh { verts, uvs, colors, indices, texture, .. }` 用 `..` 忽略 program；Text arm `NodePayload::Text { layout, font_size, color, .. }` 同样忽略。加列要把 `..` 改成显式 `program`。Unchanged 无 program 字段 → 占位 push 0。

- [ ] **Step 1: 写失败测试 — program round-trip（写 2 读 2）**

在 `blob.rs` 测试模块（`TestView` 之后）加测试。先建一个 Container+bg-image 风格的 Mesh payload（program=2）跑 round-trip：

```rust
#[test]
fn blob_program_column_round_trips() {
    // program 列（u8，第 19 列）：Mesh program=2 / Text program=1 / Unchanged program=0
    // round-trip。VERSION=5（加 program 列 bump）。
    use crate::scene::render_node::{RenderNode, NodePayload, NodeKind};
    use loomgui_core::scene::MaskContext;
    let nodes = vec![
        RenderNode {
            node_id: loomgui_core::NodeId(1),
            parent_id: None,
            visible: true,
            alpha: 1.0,
            world_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            mask_context: MaskContext(0),
            sort_key: 0,
            payload: NodePayload::Mesh {
                verts: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                uvs: vec![[0.0, 0.0]; 4],
                colors: vec![[1.0; 4]; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                texture: 0,
                program: 2,   // Container+bg-image
            },
        },
        RenderNode {
            node_id: loomgui_core::NodeId(2),
            parent_id: None,
            visible: true,
            alpha: 1.0,
            world_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            mask_context: MaskContext(0),
            sort_key: 0,
            payload: NodePayload::Unchanged,
        },
    ];
    let blob = build_blob(&nodes, &[]);
    let view = TestView::parse(&blob);
    assert_eq!(view.version(), 5, "VERSION=5（加 program 列 bump）");
    assert_eq!(view.program(0), 2, "Mesh program=2 round-trip");
    assert_eq!(view.program(1), 0, "Unchanged program=0 占位");
}
```

> 注：测试里 `RenderNode`/`NodePayload` 的确切导入路径和字段名以现有代码为准（实现者读 `blob.rs` 头部 use + `RenderNode` 定义确认）。`build_blob` 签名以现有为准（第二个参数是 clip 表，空切片）。`TestView::version()` 若不存在则加 `fn version(&self) -> u32 { u32::from_le_bytes(self.buf[4..8].try_into().unwrap()) }`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p loomgui_ffi_c blob_program_column_round_trips -- --nocapture`
Expected: 编译失败（`view.program` 方法不存在 / `col_off` 长度不匹配）或断言失败（VERSION 仍 4）。

- [ ] **Step 3: 实现 — 加 col_program 列**

`blob.rs` 改动（按顺序）：

1. `const VERSION: u32 = 4;` → `const VERSION: u32 = 5;`（行 13）。

2. columns 定义加第 19 列（行 21 起的 `let columns: &[(&str, usize)] = &[...]`）——在 `("tex_id", 4)` 后加 `("program", 1)`：
```rust
let columns: &[(&str, usize)] = &[
    ("node_id", 4), ("parent_id", 4), ("visible", 1), ("alpha", 4),
    ("sort_key", 4), ("mask_context", 4),
    ("m_a", 4), ("m_b", 4), ("m_c", 4), ("m_d", 4), ("m_tx", 4), ("m_ty", 4),
    ("payload_kind", 1), ("mesh_off", 4), ("mesh_len", 4),
    ("text_off", 4), ("text_len", 4), ("tex_id", 4),
    ("program", 1),
];
```
> 注：现有 columns 写法可能不是逐项带 size 的字面量（grep 显示是 `&[("node_id",4),...]` 风格，实现者以现有为准，只追加 `("program",1)`）。

3. 在 `col_tex_id` 声明后加 `let mut col_program = Vec::<u8>::new();`（行 57 附近）。

4. Mesh arm：`NodePayload::Mesh { verts, uvs, colors, indices, texture, .. }` → `NodePayload::Mesh { verts, uvs, colors, indices, texture, program }`，arm 内加 `col_program.push(*program as u8);`（在 `col_tex_id.extend_from_slice(...)` 后）。

> 注：`NodePayload::Mesh.program` 字段是 `u32`（mod.rs:66 `program: 0` 推断），`col_program` 是 `Vec<u8>` → 必须 `as u8`（值域 0/1/2 安全）。实现者确认 program 字段类型——若已是 u8 则去掉 `as u8`。

5. Text arm：`NodePayload::Text { layout, font_size, color, .. }` → `NodePayload::Text { layout, font_size, color, program }`，arm 内加 `col_program.push(*program as u8);`（Text program=1，已由 mod.rs 写入）。

6. Unchanged arm：加 `col_program.push(0);`（占位）。

7. `col_bufs`（行 166）末尾加 `("program", &col_program)`：
```rust
let col_bufs: Vec<(&str, &Vec<u8>)> = vec![
    ("node_id",&col_node_id),("parent_id",&col_parent_id),("visible",&col_visible),
    ("alpha",&col_alpha),("sort_key",&col_sort_key),("mask_context",&col_mask),
    ("m_a",&col_ma),("m_b",&col_mb),("m_c",&col_mc),("m_d",&col_md),
    ("m_tx",&col_mtx),("m_ty",&col_mty),
    ("payload_kind",&col_kind),("mesh_off",&col_mesh_off),("mesh_len",&col_mesh_len),
    ("text_off",&col_text_off),("text_len",&col_text_len),
    ("tex_id",&col_tex_id),
    ("program",&col_program),
];
```

- [ ] **Step 4: 扩 TestView — col_off[18]→[19] + program(i) 方法 + version()**

`TestView`（行 559 起）改动：

1. 注释 `17=tex_id （v4）` → `17=tex_id 18=program (v5)`。
2. `col_off: [usize; 18]` → `[usize; 19]`（struct 字段）。
3. `let mut col_off = [0usize; 18];` → `[0usize; 19]`（parse 内）。
4. `for i in 0..18` → `for i in 0..19`（parse 内）。
5. 加 `fn version(&self) -> u32 { u32::from_le_bytes(self.buf[4..8].try_into().unwrap()) }`。
6. 加读 program 方法：
```rust
/// 节点 i 的 program（u8 列，col_off[18] + i*1）。
fn program(&self, i: usize) -> u8 {
    self.buf[self.col_off[18] + i]
}
```

- [ ] **Step 5: 修受影响的老测试 — VERSION 断言 4→5**

grep `VERSION=4` / `version(...) == 4` / `assert_eq!(u32::from_le_bytes(blob[4..8]...), 4` 全部改 5。已知至少 3 处（行 344 `assert_eq!(v, VERSION)` 自动跟随；行 834 `assert_eq!(..., 4, "VERSION=4")`；行 858 `assert_eq!(..., 4, "VERSION=4 零 bump")`）。注释 `VERSION=4` 也改 5。

> 注：行 858 测试名含"零 bump"语义（Unchanged 无新列）——加 program 列后**有 bump**（VERSION 5），该测试断言要改 5 且注释改为"VERSION=5（加 program 列 bump）"。实现者读该测试上下文确认其断言的是 VERSION 值，改 5。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p loomgui_ffi_c`
Expected: PASS（含新 `blob_program_column_round_trips` + 修过的 VERSION 断言）。

- [ ] **Step 7: 跑全 workspace 确认无回归**

Run: `cargo test --workspace`
Expected: 全绿（446+1 测）。

- [ ] **Step 8: Commit**

```bash
git add loomgui_ffi_c/src/blob.rs
git commit -m "feat(ffi): blob 加 program 列（u8）+ VERSION 4→5

为 bg-image 共存视觉（坑 79）铺 FFI 序列化地基。
Mesh/Text program 进第 19 列；Unchanged 占位 0。
TestView col_off[18]→[19] + program(i)/version()。"
```

---

## Task 2: FrameBlob.cs 加 Program(i) + ExpectedVersion 5

**Files:**
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs`

**Interfaces:**
- Consumes: Rust blob 第 19 列 program（u8）。
- Produces: `blob.Program(i)` → `byte`，供 MirrorPool 调 `mm.Get`。

- [ ] **Step 1: 改 ExpectedVersion 4→5**

`public const uint ExpectedVersion = 4;` → `5;`（行 24）。注释 `version(u32)=4` → `=5`（行 11）。

- [ ] **Step 2: 改列注释 + 加 Program(i) 方法**

列注释（行 28-37）末尾 `17=tex_id(u32)` 后加 `18=program(u8, 0/1/2)`。

在 `TexId(i)` 方法后加：
```csharp
/// 节点 i 的 program（u8 列，ColOff(18) + i）。0=img/无图 Container，1=Text，2=Container+bg-image。
public byte Program(int i) => _buf[ColOff(18) + i];
```

> 注：ColOff 索引 18 对应第 19 列（0-based）。ColOff 方法本身不用改（它读 `12 + idx*4` 的 header offset，idx=18 自动读第 19 个 col_offset）。但要确认 header 长度注释（`12+18*4` 等算式）里"18"是列数常量——grep 确认这些是字面量 18，要全改成 19（mesh_arena_off @ `12+18*4`、text @ `12+18*4+2*4`、clip @ `12+18*4+4*4`）。**这是关键**：header 现在有 19 个 col_offset，arena 段 offset 起点要后移 4 字节。

- [ ] **Step 3: 修 header offset 算式（18→19）**

FrameBlob.cs 里所有 `12 + 18 * 4` 算式改 `12 + 19 * 4`（MeshArenaOff/TextArenaOff/ClipTableOff 等）。注释里的 `@ 12+18*4=84` 等也改（`12+19*4=88`）。

> 这是 Rust 侧 `blob.rs` 行 29-31 的 `num_col_offsets = columns.len()`（现 19）自动驱动的——Rust 侧 header 写 19 个 col_offset，C# 侧读算式必须同步 19。

- [ ] **Step 4: 手动核对（无 Unity 编译环境时的静态确认）**

本机无 Unity 编译，实现者改完后 grep 确认：
- `ExpectedVersion` = 5
- `Program(int i)` 方法存在，读 `ColOff(18) + i`
- 所有 `18 * 4` header 算式改 `19 * 4`
- 列注释含 `18=program(u8)`

Run: `grep -n "18 \* 4\|ExpectedVersion\|Program(int" loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs`
Expected: 无残留 `18 * 4`；`ExpectedVersion = 5`；`Program(int i)` 存在。

- [ ] **Step 5: Commit**

```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs
git commit -m "feat(unity): FrameBlob 加 Program(i) + ExpectedVersion 5

镜像 Rust blob program 列（u8，第 19 列）。
header col_offset 18→19，arena 段 offset 算式同步。"
```

---

## Task 3: core Container+bg-image 分支 program=2

**Files:**
- Modify: `loomgui_core/src/render/mod.rs`

**Interfaces:**
- Consumes: `n.style.background_image`（`Option<String>`）、`textures`（`TextureRegistry`，`.get(url)`）。
- Produces: Container 分支 `NodePayload::Mesh { ..., program }` = 2（有图命中）或 0（无图）。

**现状关键**：行 166-168 Container 分支 `program: 0` 硬写。行 128-137 已算出 `texture`（命中=真 tex_id，未注册/无图=0）。

- [ ] **Step 1: 写失败测试 — Container+bg-image 命中 → program=2**

在 `render/mod.rs` 测试模块（§Container bg-image 区，行 989 起）加：

```rust
#[test]
fn build_container_with_bg_image_hit_sets_program_2() {
    // Container 设 background-image 且纹理命中 → program=2（CSS 合成）。
    let mut tex = TextureRegistry::default();
    let _ = tex.register("a.png".into(), image_region_default());
    let mut scene = one_container_scene_with_bg_image("a.png");
    crate::scene::transform::compute_world_transforms(&mut scene);
    let (frame, _) = build_render_nodes(&scene, &font_for_test(), &tex, &[]);
    match &frame.nodes[0].payload {
        NodePayload::Mesh { program, texture, .. } => {
            assert_ne!(*texture, 0, "命中纹理 tex_id≠0");
            assert_eq!(*program, 2, "Container+bg-image 命中 → program=2");
        }
        _ => panic!("expected Mesh"),
    }
}

#[test]
fn build_container_without_bg_image_keeps_program_0() {
    // Container 无 bg-image → program=0（tex*vcol，白占位×bg-color=bg-color）。
    let mut scene = one_container_scene_no_bg_image();
    crate::scene::transform::compute_world_transforms(&mut scene);
    let (frame, _) = build_render_nodes(&scene, &font_for_test(), &TextureRegistry::default(), &[]);
    match &frame.nodes[0].payload {
        NodePayload::Mesh { program, .. } => {
            assert_eq!(*program, 0, "无 bg-image → program=0");
        }
        _ => panic!("expected Mesh"),
    }
}

#[test]
fn build_container_bg_image_unregistered_keeps_program_0() {
    // Container 设 bg-image 但纹理未注册（哨兵 tex_id=0）→ program=0（不走合成，白占位）。
    let mut scene = one_container_scene_with_bg_image("missing.png");
    crate::scene::transform::compute_world_transforms(&mut scene);
    let (frame, _) = build_render_nodes(&scene, &font_for_test(), &TextureRegistry::default(), &[]);
    match &frame.nodes[0].payload {
        NodePayload::Mesh { program, texture, .. } => {
            assert_eq!(*texture, 0, "未注册 → tex_id=0 哨兵");
            assert_eq!(*program, 0, "未注册 → program=0（不走合成）");
        }
        _ => panic!("expected Mesh"),
    }
}

#[test]
fn build_image_node_keeps_program_0() {
    // Image 节点 program=0（tex*vcol，图透明区透下层）——零改回归。
    let mut tex = TextureRegistry::default();
    let _ = tex.register("a.png".into(), image_region_default());
    let mut scene = one_image_scene("a.png");
    crate::scene::transform::compute_world_transforms(&mut scene);
    let (frame, _) = build_render_nodes(&scene, &font_for_test(), &tex, &[]);
    let img = frame.nodes.iter().find(|n| matches!(&n.payload, NodePayload::Mesh { .. }))
        .expect("img mesh");
    if let NodePayload::Mesh { program, .. } = &img.payload {
        assert_eq!(*program, 0, "Image → program=0（零改）");
    }
}
```

> 注：helper `one_container_scene_with_bg_image`/`one_container_scene_no_bg_image`/`one_image_scene`/`image_region_default`/`font_for_test` —— 实现者复用现有测试 helper（行 989+ 已有 `build_container_with_bg_image_uses_tex_id_and_fit_uv` 等测试，照其 setup 模式构造；若 helper 不存在就内联构造，参考 `container_node` helper 行 319）。关键是 setup 与现有 `build_container_with_bg_image_uses_tex_id_and_fit_uv`（行 992）一致，只多断言 program。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p loomgui_core build_container_with_bg_image_hit_sets_program_2`
Expected: FAIL（`program=0`，断言期望 2）。

- [ ] **Step 3: 实现 — Container 分支 program 条件赋值**

`render/mod.rs` 行 166-168，把 `program: 0` 改为条件值。在 match `background_image` 那段（行 128-137）已算出 `texture`——用它判断命中（`texture != 0` 即命中，因为未注册/无图都返回 0）：

```rust
// program：Container+bg-image 命中纹理 → 2（CSS 合成）；否则 0（tex*vcol）。
let program = if texture != 0 { 2u32 } else { 0u32 };
rn.payload = NodePayload::Mesh {
    verts: v, uvs: uvc, colors: col, indices: idx, texture, program,
};
```

> 注：`texture` 是 `u32`（tex_id），命中时非 0。无图分支返回 `0u32`，未注册也返回 `0u32`——两者都 → program=0，符合 spec §3.1。Image 分支（行 181）保持 `program: 0` 不动。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p loomgui_core build_container_with_bg_image`
Expected: 4 个新测全 PASS + 旧 `build_container_with_bg_image_uses_tex_id_and_fit_uv` 仍 PASS。

- [ ] **Step 5: 跑全 workspace**

Run: `cargo test --workspace`
Expected: 全绿。

- [ ] **Step 6: Commit**

```bash
git add loomgui_core/src/render/mod.rs
git commit -m "feat(core): Container+bg-image 命中纹理 → program=2

CSS 合成分支。无图/未注册保持 program=0。
Image 保持 program=0。pkg.bin 零改（program 是 frame blob 字段）。"
```

---

## Task 4: shader BG_COMPOSITE 分支 + MaterialManager keyword + MirrorPool 用 blob.Program

> Unity 侧三处一起改（shader/C#/MirrorPool），无独立 Rust 测试，靠 PlayMode 验收。本机改完只能静态核对，家里机 PlayMode 验收。

**Files:**
- Modify: `loomgui_unity/Assets/LoomGUI/Shaders/LoomGUI-Unlit.shader`
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs`
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`

**Interfaces:**
- Consumes: `blob.Program(i)`（Task 2 产）、`MaterialManager.Get(program, ...)`。
- Produces: program=2 节点走 BG_COMPOSITE 合成 frag。

- [ ] **Step 1: shader 加 multi_compile + frag 分支**

`LoomGUI-Unlit.shader` Pass 段（行 39 起 `#pragma multi_compile _ ALPHA_MASK` 后）加：
```hlsl
#pragma multi_compile _ BG_COMPOSITE
```

frag 函数里找现有 `half4 col = tex * vcol;`（或等价的 `tex2D(_MainTex, i.uv) * i.color`），改为：
```hlsl
half4 tex = SAMPLE_TEXTURE2D(_MainTex, sampler_MainTex, i.uv);
half4 col;
#if defined(BG_COMPOSITE)
    // CSS background 合成：图不透明区显图，透明区显 bg-color（vcol）。
    col.rgb = tex.rgb * tex.a + i.color.rgb * (1.0 - tex.a);
    col.a = i.color.a;
#else
    col = tex * i.color;
#endif
```

> 注：实现者读现有 frag 确认 tex 采样变量名 + vcol 来源（`i.color`）。ALPHA_MASK（text）分支若已 `#if defined(ALPHA_MASK)` 单独处理，BG_COMPOSITE 与之互斥（program 2≠1），不会同时定义——但实现者要确认 frag 里 ALPHA_MASK 分支的 `#else` 不会吞掉 BG_COMPOSITE。最稳：BG_COMPOSITE 分支放在最外层 `#if defined(ALPHA_MASK) ... #elif defined(BG_COMPOSITE) ... #else ... #endif`。

- [ ] **Step 2: MaterialManager 加 program==2 keyword**

`MaterialManager.cs` 行 37（`if (program == 1) mat.EnableKeyword("ALPHA_MASK");` 后）加：
```csharp
if (program == 2) mat.EnableKeyword("BG_COMPOSITE");   // Container+bg-image: CSS 合成
```

- [ ] **Step 3: MirrorPool 用 blob.Program(i) 替换硬编码 0**

`MirrorPool.cs` 行 129（Mesh 路径）：
```csharp
var mat = mm.Get(program: 0, tex, maskCtx, !pure);
```
改为：
```csharp
var mat = mm.Get((int)blob.Program(i), tex, maskCtx, !pure);
```

> 注：Text 路径（行 156 `mm.Get(program: 1, ...)`）保持硬编码 1 不动——Text 的 program 恒 1，且 blob 也会写 1（Task 1 Text arm push *program=1），两者一致；但为统一可也改 `blob.Program(i)`。**推荐保持 Text 硬编码 1**（YAGNI，Text 路径不涉及 bg-image，少一处耦合）。实现者定。

- [ ] **Step 4: 静态核对（本机无 Unity 编译）**

grep 确认：
Run: `grep -rn "BG_COMPOSITE" loomgui_unity/Assets/LoomGUI/`
Expected: shader（multi_compile + frag 分支）+ MaterialManager（keyword）共 ≥3 处。

Run: `grep -n "blob.Program\|program: 0" loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`
Expected: Mesh 路径用 `blob.Program(i)`；无残留 `program: 0`（Text 路径 `program: 1` 保留）。

- [ ] **Step 5: Commit**

```bash
git add loomgui_unity/Assets/LoomGUI/Shaders/LoomGUI-Unlit.shader \
        loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs \
        loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs
git commit -m "feat(unity): BG_COMPOSITE 合成分支 + program 驱动 Material

shader: #if BG_COMPOSITE 走 CSS 合成公式（图透明区显 bg-color）。
MaterialManager: program==2 EnableKeyword BG_COMPOSITE。
MirrorPool: Mesh 路径用 blob.Program(i) 替换硬编码 0。
兑现 v1.1 §6.2 Container+bg-image 共存视觉。"
```

---

## Task 5: 重编 loomgui_ffi_c.dll + push（controller 本机）

> 本机 build，家里机才能 PlayMode 测。pkg.bin **不重打**（program 是 frame blob 字段，pkg.bin 存 scene/style 不存 frame blob；v1.2 pkg.bin v9 不变）。

**Files:**
- Modify: `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`（重编产物）

- [ ] **Step 1: 重编 .dll**

Run: `cargo build --release -p loomgui_ffi_c`
Expected: 编译成功，产物在 `target/release/loomgui_ffi_c.dll`。

- [ ] **Step 2: cp 到 Unity Plugins**

Run: `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`
Expected: 文件更新（大小变，含 program 列 + VERSION 5）。

- [ ] **Step 3: 跑 core 全测确认无回归**

Run: `cargo test --workspace`
Expected: 全绿。

- [ ] **Step 4: Commit + push**

```bash
git add loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll
git commit -m "build(unity): 重编 dll 含 bg-image 共存视觉修复（v5 blob）

含 blob program 列 + Container+bg-image program=2。
pkg.bin 不变（v9，frame blob 字段不入 pkg）。
家里机 PlayMode 验收 §3.6 + §3.7 R5。"
git push
```

- [ ] **Step 5: 通知家里机验收**

家里机 pull 后 PlayMode 验收（spec §6）：
- §3.6 第 2 行（青 + home.png contain）：图外区显青底，icon 显图。
- §3.6 第 3 行（红 + 100%）：红底 + 拉伸图。
- §3.6 第 1 行（深蓝 + cover）：深蓝底 + 图。
- §3.7 R5（图+圆角共存）回归：圆角镂空区透下层，圆角内图透明区显 bg-color。

---

## 验收清单（家里机 PlayMode）

- [ ] §3.6 三行：图透明区显对应 bg-color（青/红/深蓝），非透 root。
- [ ] §3.7 R5 图+圆角共存：圆角镂空区透下层（不显 bg-color 方块），圆角内合成正确。
- [ ] img 节点零回归（§3.6 第 1 行 icon、各处 Image）。
- [ ] text 零回归（ALPHA_MASK 路径）。
- [ ] 无 bg-image 的 Container 零回归（纯色块显 bg-color）。
