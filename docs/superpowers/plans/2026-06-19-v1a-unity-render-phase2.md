# v1a Phase 2 渲染补全 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 v1a 关到验收门——在已通的 Unity 渲染管线上补完 Text、rect mask、500 节点压测、Domain reload，让 v0 fixture（div flex + 文本 + rect mask）在 Unity 真渲、500 节点静态无卡顿、进出 Play 不 crash。

**Architecture:** 核心 Rust 已产 TextLayout / clip_rect / mask_context（v0 就绪），Phase 2 = FFI 序列化 + Unity 接线。Blob v1→v2（加 text_arena + clip 表）。Text：Rust 笔位权威 + Unity 纯光栅（`GetCharacterInfo` UV/box + `textureRebuilt` 监听），偏离 fgui 的 advance/行高。rect mask：Rust 嵌套交集 + Unity `_ClipBox` per-context material。顺带修 latent 坐标 bug（MirrorPool 巢状+绝对 localPosition 双计父 → flatten）。

**Tech Stack:** Rust（loomgui_core + loomgui_ffi_c，csbindgen）/ Unity 6.5 (6000.5.0f1) URP 17.5 / Mono backend / C# / DejaVuSans.ttf。

## Global Constraints

- **Unity 6.5** (6000.5.0f1)，URP 17.5.0，Mono backend，LoomUI layer index 6。
- **参考 fgui（★ 准则）**：实现任何机制前先读 `temp/FairyGUI-unity/`。Text → `Assets/Scripts/Core/Text/DynamicFont.cs`（DrawGlyph/GetGlyph/textureRebuildCallback）；clip → `Assets/Scripts/Core/UpdateContext.cs:105-156`（EnterClipping/_ClipBox）+ `NGraphics.cs`（material 取）。
- **坐标**：blob 的 local_x/local_y 与 clip_rect 均为**绝对 design 坐标**（layout 累加 parent origin）。后端 flatten 挂根 GO。
- **Blob v2**：magic `0x4D4F4F4C`，version `2`。C# 必须校验 magic + version。
- **Text 契约**：Unity 仅光栅（UV/box 从 `GetCharacterInfo(char, font_size)`，不用 advance/行高）；笔位严格按 Rust（pen_y = line.y + line.baseline，content 偏移 Rust 烤进）。
- **csbindgen**：生成 `internal` Native/StageHandle（LoomGUI.Bindings asmdef），LoomGUI.Runtime 经 `[InternalsVisibleTo]` 访问（已配）。
- **字体**：DejaVuSans.ttf（仓库 `loomgui_core/tests/fixtures/`，Unity 拷到 `Assets/StreamingAssets/`，已就位）。
- **测试**：Rust `cargo test`；Unity EditMode `[Test]`（NUnit）；PlayMode 视觉验收**手动**（用户看）。
- **EditMode-safe 销毁**：`Application.isPlaying ? Object.Destroy : Object.DestroyImmediate`（LoomStage 挂 `[ExecuteAlways]`）。
- **TDD**：每 task 先写失败测试 → 验红 → 实现 → 验绿 → commit。无 placeholder。

## File Structure

**Rust（loomgui_core）**：
- `src/text/layout.rs` — `Glyph` 加 `codepoint` 字段（T3）。
- `src/render/batch.rs` — DFS 算嵌套 clip 交集（T5）。
- `src/render/mod.rs` — Text payload 烤 content 偏移（T3）。

**Rust FFI（loomgui_ffi_c）**：
- `src/blob.rs` — Blob v2（13 列 + text_arena + clip 表，version 2）（T1/T3/T5）。

**Unity（loomgui_unity/Assets/LoomGUI/Runtime）**：
- `FrameBlob.cs` — v2 解析（13 列 + text_arena + clip 表 + magic/version 校验）（T1）。
- `MirrorPool.cs` — flatten 修（T2）；Text mesh 上传（T4）；clip material 接线（T6）。
- `MaterialManager.cs` — SetClipBox/CLIPPED 接线（T6）。
- `TextRasterizer.cs`（新）— RequestCharacters/GetCharacterInfo/textureRebuild（T4）。
- `LoomStage.cs` — text font 注入、Domain reload 接线（T4/T8）。
- `Shaders/LoomGUI-Unlit.shader` — 已有 CLIPPED variant（Phase 1），T6 验。

**Test（Rust）**：`loomgui_ffi_c/src/blob.rs` 内 `#[cfg(test)]`（TestView 扩展）。
**Test（Unity EditMode）**：`Assets/LoomGUI/Tests/`（`LoomGUI.Tests.asmdef`，已就位）。

---

### Task 1: Blob v2 scaffold（header + 13 列 + text/clip arena 占位 + version 校验）

**Files:**
- Modify: `loomgui_ffi_c/src/blob.rs`
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs`
- Test: `loomgui_ffi_c/src/blob.rs` (TestView)；`Assets/LoomGUI/Tests/FrameBlobV2Tests.cs`

**Interfaces:**
- Produces: blob header v2（88B：magic/version=2/node_count + 13 col_offset + mesh/text/clip arena off+len）；FrameBlob v2 解析 + magic/version 校验。Text/clip 暂 emit 空（T3/T5 填）。
- Consumes: 现有 RenderNode（payload_kind 1=Mesh 仍走 mesh_arena；2=Text 暂空；新增 clip 表暂 count=0）。

- [ ] **Step 1: Rust — bump version + 改 columns + header**

`loomgui_ffi_c/src/blob.rs`：
```rust
const VERSION: u32 = 2;   // v1→v2
// ...
let columns: &[(&str, usize)] = &[
    ("node_id", 4), ("parent_id", 4), ("visible", 1), ("alpha", 4),
    ("sort_key", 4), ("local_x", 4), ("local_y", 4), ("mask_context", 4),
    ("payload_kind", 1), ("mesh_off", 4), ("mesh_len", 4),
    ("text_off", 4), ("text_len", 4),   // v2 新增
];
let num_col_offsets = columns.len();      // 13
let header_len = 3 * 4                    // magic, version, node_count
    + num_col_offsets * 4                 // 13 col offset
    + 2 * 4                               // mesh_arena off+len
    + 2 * 4                               // text_arena off+len   (v2 新增)
    + 2 * 4;                              // clip_table off+len   (v2 新增)
```
新增 text_arena + clip_table 收集 + per-node col_text_off/col_text_len。Text 节点暂 emit 空 text（text_off=0/text_len=0），clip 表暂 `clip_count=0`。拼装时追加 text_arena 段 + clip 表段，header 写各自 off/len。

- [ ] **Step 2: Rust — 失败测试（version + 13 列 + 三 arena header）**

`blob.rs` tests 加：
```rust
#[test]
fn blob_v2_header_has_text_and_clip_arena_fields() {
    let blob = build_blob(&[mesh_node(0, None, 0.0,0.0, 1.0,1.0)]);
    assert_eq!(u32::from_le_bytes(blob[4..8].try_into().unwrap()), 2, "version=2");
    // 13 col offset @ [12..12+13*4)
    let n_cols = u16::try_from(13u32).unwrap();
    let _ = n_cols;
    // text_arena_off @ 12+13*4+2*4 = 72；text_arena_len @ 76
    let text_off = u32::from_le_bytes(blob[72..76].try_into().unwrap());
    let text_len = u32::from_le_bytes(blob[76..80].try_into().unwrap());
    assert_eq!(text_len, 0, "T1: text_arena 暂空");
    // clip_table_off @ 80；clip_table_len @ 84
    let clip_len = u32::from_le_bytes(blob[84..88].try_into().unwrap());
    assert_eq!(clip_len, 4, "T1: clip 表至少含 clip_count(u32)=0");
    let clip_count = u32::from_le_bytes(blob[text_off as usize..text_off as usize+4].try_into().unwrap());
    assert_eq!(clip_count, 0);
}
```
另：现有 `build_blob_has_magic_and_count` 改断言 version==2。

- [ ] **Step 3: Rust — run test, 验红再绿**

`cargo test -p loomgui_ffi_c` → 先红（version=1），实现 Step 1 后绿。

- [ ] **Step 4: C# — FrameBlob v2 解析 + magic/version 校验**

`FrameBlob.cs`：列数 11→13（`ColOff` 注释 + `TextOff`/`TextLen` 访问器）；header offset 重算（13 列 → text_arena @ 12+13*4+8=72，clip @ 80）；加 `Magic`/`Version` 校验（ctor 不抛——加 `bool IsValid` 属性：magic==Magic && version==2）。
```csharp
public bool IsValid => ReadU32(0) == Magic && ReadU32(4) == 2;
public uint Version => ReadU32(4);
int TextArenaOff => (int)ReadU32(12 + 13*4 + 2*4);       // 72
int TextArenaLen => (int)ReadU32(12 + 13*4 + 2*4 + 4);   // 76
int ClipTableOff => (int)ReadU32(12 + 13*4 + 4*4);       // 80
int ClipTableLen => (int)ReadU32(12 + 13*4 + 4*4 + 4);   // 84
public uint TextOff(int i) => ReadU32(ColOff(11) + i*4);
public uint TextLen(int i) => ReadU32(ColOff(12) + i*4);
public int ClipCount => ClipTableLen >= 4 ? (int)ReadU32(ClipTableOff) : 0;
// ClipRect(ctx): 读 clip 表 entry ctx → {context_id, x,y,w,h}
public bool ClipRect(int ctx, out float x, out float y, out float w, out float h);
```

- [ ] **Step 5: C# — EditMode 测试（magic/version 校验 + 现有 mesh 仍解析）**

`FrameBlobV2Tests.cs`：构造 v2 blob（可从 Rust 产的 bytes，或手搓最小 v2 header）→ `IsValid==true`、`Version==2`、`ClipCount==0`、单 mesh 节点 `ReadMesh` 仍正确。

- [ ] **Step 6: run + commit**

`cargo test` 全绿；Unity EditMode 测试绿。`MirrorPool.Sync` 开头加 `if (!blob.IsValid) return;`（防误读 v1 blob）。
```bash
git add loomgui_ffi_c/src/blob.rs loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs loomgui_unity/Assets/LoomGUI/Tests/FrameBlobV2Tests.cs loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs
git commit -m "feat(v1a-p2): Blob v2 scaffold — 13列 + text/clip arena header + magic/version 校验"
```

---

### Task 2: 多节点坐标 flatten 修 + sort_key 视觉验证

**Files:**
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`
- Test: `Assets/LoomGUI/Tests/MirrorPoolFlattenTests.cs`（EditMode）

**Interfaces:**
- Consumes: FrameBlob（local_x/local_y 绝对 design，parent_id）。
- Produces: 所有渲染对象挂根 GO（flatten），localPosition=绝对；`parent_id` 渲染不用（保留 blob 列）。

**参考**：spec §4.2。当前 `MirrorPool.cs:49-54` 巢状 SetParent + 绝对 localPosition → 多节点双计父。

- [ ] **Step 1: 失败测试（flatten：子节点位置不被父偏移）**

`MirrorPoolFlattenTests.cs`：构造 2 节点 blob（parent @ design (100,200)，child @ design (50,50)，parent_id 链），Sync 后断言 child GO 的 **world** position == 根 transform 映射 (50,50)（不是 (150,250)）。用根 GO scale=(1,-1,1) pos=(0,0) 简化断言：child world.y == -50（若 design y=50 → world y=-50）。
```csharp
[Test] public void Flatten_ChildWorldPos_NotOffsetByParent() {
    // 建 root GO (scale 1,-1,1)，2 节点 blob，Sync，断言 child.transform.position == expected(50,-50,...)
}
```

- [ ] **Step 2: run，验红**（当前巢状实现 child world = (150,-250) ≠ (50,-50)）

- [ ] **Step 3: 实现 flatten**

`MirrorPool.cs` Sync 循环内，替换巢状块：
```csharp
// flatten：所有节点挂根 GO，localPosition=绝对 design（避免巢状双计父）。
// parent_id 渲染不用（v1c 事件再用）。
ro.Go.transform.SetParent(root, false);
ro.Go.transform.localPosition = new Vector3(blob.LocalX(i), blob.LocalY(i), 0f);
ro.Go.transform.localScale = Vector3.one;
```
（删掉 parent_id 查找 + SetParent(pro) 块。）

- [ ] **Step 4: run，验绿**

- [ ] **Step 5: PlayMode 视觉（手动，用户验）**

LoomStage `_html`/`_css` 改多节点（如 3 个不同色块 + flex 堆叠）→ Play → 验位置/堆叠/绘制序对。用户报结果。

- [ ] **Step 6: commit**
```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs loomgui_unity/Assets/LoomGUI/Tests/MirrorPoolFlattenTests.cs
git commit -m "fix(v1a-p2): MirrorPool flatten 挂根 GO——修巢状+绝对 localPosition 双计父（Phase 1 单节点未暴露）"
```

---

### Task 3: Text Rust emit（Glyph+codepoint + text_arena 序列化）

**Files:**
- Modify: `loomgui_core/src/text/layout.rs`（Glyph 加 codepoint）
- Modify: `loomgui_core/src/render/mod.rs`（Text payload 烤 content 偏移）
- Modify: `loomgui_ffi_c/src/blob.rs`（Text 节点序列化进 text_arena）
- Test: `loomgui_ffi_c/src/blob.rs` TestView 扩展

**Interfaces:**
- Consumes: `NodePayload::Text { layout, font_size, color, program }`（v0 已 emit）。
- Produces: text_arena per text node = `{ font_size:u32, color:f32×4, glyph_count:u32, glyphs[{codepoint:u32, pen_x:f32, pen_y:f32}] }`，pen = GO-local（content 偏移已烤）。

**参考**：spec §4.1/§4.3。Unity `GetCharacterInfo(char)` 要码点（非 glyph_id）。

- [ ] **Step 1: Glyph 加 codepoint**

`layout.rs`：
```rust
pub struct Glyph {
    pub glyph_id: u16,
    pub codepoint: u32,   // v1a 新增：Unity GetCharacterInfo 按 char
    pub x: f32, pub y: f32, pub bearing_x: f32, pub bearing_y: f32,
}
```
`measure_text` 遍历 `content.chars()` 时填 `codepoint: c as u32`（同时 `glyph_id` 由 ttf `glyph_id(char)` 查）。

- [ ] **Step 2: render/mod.rs 烤 content 偏移**

建 Text payload 前，从 ResolvedStyle 取 `padding_left+border_left`、`padding_top+border_top`（content 偏移），加到 layout 各 glyph 的 (x, y)（让 pen = GO-local = layout_rect 原点相对）。若 ResolvedStyle 无 padding/border 字段，Phase 2 用 (0,0)（记 ledger：text 节点暂不支持 border/padding 内偏移，v1b 补）。
> 实现者先 grep `ResolvedStyle` 确认 padding/border 字段名；有则烤，无则 (0,0)+注释。

- [ ] **Step 3: blob.rs 序列化 Text 进 text_arena**

`build_blob` Text 分支（替换 T1 的空 emit）：
```rust
NodePayload::Text { layout, font_size, color, .. } => {
    col_kind.push(2);
    let seg_off = text_arena.len() as u32;
    text_arena.extend_from_slice(&(*font_size as u32).to_le_bytes());
    for &c in color { text_arena.extend_from_slice(&c.to_le_bytes()); }   // f32×4
    let glyphs_start = text_arena.len();
    text_arena.extend_from_slice(&0u32.to_le_bytes()); // glyph_count 占位
    let mut count = 0u32;
    for line in &layout.lines {
        let pen_y = line.y + line.baseline;   // 绝对（content 偏移 Step 2 已加进 glyph.y）
        for run in &line.runs {
            for g in &run.glyphs {
                text_arena.extend_from_slice(&g.codepoint.to_le_bytes());
                text_arena.extend_from_slice(&g.x.to_le_bytes());
                text_arena.extend_from_slice(&pen_y.to_le_bytes()); // 同行同 pen_y
                count += 1;
            }
        }
    }
    text_arena[glyphs_start..glyphs_start+4].copy_from_slice(&count.to_le_bytes());
    let seg_len = text_arena.len() as u32 - seg_off;
    col_text_off.extend_from_slice(&seg_off.to_le_bytes());
    col_text_len.extend_from_slice(&seg_len.to_le_bytes());
    col_mesh_off.extend_from_slice(&0u32.to_le_bytes());
    col_mesh_len.extend_from_slice(&0u32.to_le_bytes());
}
```

- [ ] **Step 4: 失败测试（TestView 读 text_arena round-trip）**

`blob.rs` TestView 加 `read_text(i) -> {font_size, color, glyphs[]}`；测试：构造 1 text 节点（"AB"，font_size=24，color=红）→ 读回 glyph_count==2、codepoint=='A'(65)/'B'(66)、font_size==24、color 正确。

- [ ] **Step 5: run，验红→绿**

`cargo test -p loomgui_ffi_c` + `cargo test -p loomgui_core`（measure_text 仍绿）。

- [ ] **Step 6: commit**
```bash
git add loomgui_core/src/text/layout.rs loomgui_core/src/render/mod.rs loomgui_ffi_c/src/blob.rs
git commit -m "feat(v1a-p2): Text Rust emit — Glyph+codepoint + text_arena 序列化（font_size/color/glyphs）"
```

---

### Task 4: Text Unity 光栅（RequestCharacters/GetCharacterInfo/textureRebuild）

**Files:**
- Create: `loomgui_unity/Assets/LoomGUI/Runtime/TextRasterizer.cs`
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`（kind=2 → gen text mesh）
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs`（注入 font + textureRebuild 监听）
- Test: `Assets/LoomGUI/Tests/TextRasterizerTests.cs`（EditMode）

**Interfaces:**
- Consumes: FrameBlob text_arena（T3）；`Font`（Unity 动态字体，DejaVu）。
- Produces: text 节点 Mesh（glyph quads），texture=font atlas，program=1。

**参考**：fgui `DynamicFont.cs`（PrepareCharacters/GetGlyph/DrawGlyph/textureRebuildCallback）、`Stage.cs:828`（textRebuildFlag 重跑帧）。**偏离**：笔位用 Rust（不用 `_char.advance`）；行高用 Rust（不用 `fontSize*1.25`）。

- [ ] **Step 1: TextRasterizer — 失败测试（glyph quad 数 + 笔位）**

`TextRasterizerTests.cs`：mock/minimal：给 font（DejaVu）+ 一组 glyphs（"A" @ pen (0, baseline)），`BuildMesh` 返回 vertCount==4（1 quad）/idxCount==6，且首顶点位置 ≈ pen + box。EditMode 用 `Font.RequestCharactersInTexture` + `GetCharacterInfo`（需 font 动态；DejaVu 在 StreamingAssets，EditMode 可 `new Font()` + `Font.material.mainTexture`）。
```csharp
[Test] public void BuildMesh_OneGlyph_ProducesOneQuad() {
    var font = LoadDejaVu();
    var glyphs = new[]{ new GlyphData('A', 0f, 20f) };
    var mesh = TextRasterizer.BuildMesh(font, 24, Color.white, glyphs);
    Assert.AreEqual(4, mesh.Verts.Length);
    Assert.AreEqual(6, mesh.Idx.Length);
}
```

- [ ] **Step 2: run，验红**（TextRasterizer 不存在）

- [ ] **Step 3: 实现 TextRasterizer.BuildMesh**

```csharp
public static MeshSegment BuildMesh(Font font, int fontSize, Color color, ReadOnlySpan<GlyphData> glyphs) {
    // 1. 收集 codepoints → RequestCharactersInTexture
    var sb = new StringBuilder();
    foreach (var g in glyphs) sb.Append((char)g.Codepoint);
    font.RequestCharactersInTexture(sb.ToString(), fontSize, FontStyle.Normal);
    // 2. 每 glyph → quad
    var verts = new List<Vector3>(); var uvs = new List<Vector2>();
    var cols = new List<Color>(); var idx = new List<int>();
    int vi = 0;
    foreach (var g in glyphs) {
        if (!font.GetCharacterInfo((char)g.Codepoint, out var info, fontSize, FontStyle.Normal)) continue;
        float pl = g.PenX + info.minX, pr = g.PenX + info.maxX;
        float pt = g.PenY - info.maxY, pb = g.PenY - info.minY;  // y-down：maxY 在基线上方→减
        // quad：BL,TL,TR,BR（与 fgui DrawGlyph 顶点序一致）
        verts.Add(new Vector3(pl, pb, 0)); verts.Add(new Vector3(pl, pt, 0));
        verts.Add(new Vector3(pr, pt, 0)); verts.Add(new Vector3(pr, pb, 0));
        uvs.Add(info.uvBottomLeft); uvs.Add(info.uvTopLeft);
        uvs.Add(info.uvTopRight); uvs.Add(info.uvBottomRight);
        for (int k=0;k<4;k++) cols.Add(color);
        idx.Add(vi); idx.Add(vi+1); idx.Add(vi+2); idx.Add(vi); idx.Add(vi+2); idx.Add(vi+3);
        vi += 4;
    }
    return new MeshSegment(verts, uvs, cols, idx);  // 复用 MeshSegment 或新 TextMeshSegment
}
```
> `GlyphData{int Codepoint;float PenX,PenY}` 由 FrameBlob.ReadText 填。

- [ ] **Step 4: textureRebuild 监听**

`TextRasterizer` static：`static int s_fontVersion;` + `static void OnRebuilt(Font f){ s_fontVersion++; }`。`LoomStage.Awake`：`Font.textureRebuilt += TextRasterizer.OnRebuilt`；`OnDestroy`：解绑。MirrorPool.Sync 开头：若 `TextRasterizer.ConsumedVersion != s_fontVersion`，强制所有 text 节点 dirty 重光栅（fgui Stage.cs:828 重跑帧语义——这里标记 text 节点需重建 mesh）。
> 最小实现：MirrorPool 记 `int _lastFontVersion`；Sync 时若不等，把池中 text 节点标记需重 BuildMesh。

- [ ] **Step 5: MirrorPool kind=2 接线**

Sync 循环：`byte kind = blob.PayloadKind(i);` 后，kind==1 走现有 mesh；kind==2：
```csharp
if (kind == 2) {
    blob.ReadText(i, out var fontSize, out var color, out var glyphs);
    var seg = TextRasterizer.BuildMesh(_font, fontSize, color * nodeAlpha, glyphs);
    UploadMesh(ro, seg);  // 复用 UploadMesh（接受 MeshSegment）
    ro.Mr.sharedMaterial = mm.Get(program: 1, _font.material.mainTexture, blob.MaskContext(i));
    // transform/sortingOrder 同 mesh 分支
}
```
重构 Sync：transform/sortingOrder 设置提到 kind 分支前（两 kind 共用）。

- [ ] **Step 6: run EditMode，验绿**

- [ ] **Step 7: PlayMode 视觉（手动，用户验）**

LoomStage `_html`/`_css` 加 text 节点（如 `<div class="t">Hello</div>` + `.t{color:#fff;font-size:48px;}`）→ Play → 验 ASCII 文本渲出、位置/基线对。用户报。

- [ ] **Step 8: commit**
```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/TextRasterizer.cs loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs loomgui_unity/Assets/LoomGUI/Tests/TextRasterizerTests.cs
git commit -m "feat(v1a-p2): Text Unity 光栅 — RequestCharacters/GetCharacterInfo（Rust 笔位）+ textureRebuild 监听"
```

---

### Task 5: rect mask Rust（嵌套 clip 交集 + clip 表 emit）

**Files:**
- Modify: `loomgui_core/src/render/batch.rs`
- Modify: `loomgui_ffi_c/src/blob.rs`（clip 表填充）
- Test: `loomgui_core/src/render/batch.rs`

**Interfaces:**
- Consumes: 节点 `clip_rect`（绝对 design，layout 已填）。
- Produces: clip 表 `entries[{context_id, x,y,w,h}]`，rect = 祖先 clip 链交集（绝对 design）。

**参考**：fgui `UpdateContext.cs:109-110`（嵌套 `ToolSet.Intersection`）。

- [ ] **Step 1: 失败测试（嵌套不相交 → 交集空/小）**

`batch.rs` tests：构 scene：outer clip [0,0,100,100] → mid clip [200,200,50,50]（不相交）→ leaf。assert leaf 的 mask_context 对应的 clip rect == 交集（空或零面积）。当前 v0 batch 只赋新 context 不交 → leaf rect == mid box [200,200,50,50]（错，应交集空）。

- [ ] **Step 2: run，验红**

- [ ] **Step 3: batch.rs DFS 算交集**

DFS 维护 `accumulated: Option<Rect>`（祖先 clip 链交）。遇 `clip_rect.is_some()` 节点：
```rust
let own = node.clip_rect.unwrap();
let intersected = match accumulated {
    None => Some(own),
    Some(a) => Some(rect_intersect(a, own)),   // 空交集返回零面积 Rect
};
let ctx = next_context_id();   // 计数器+1
clip_table.push((ctx, intersected.unwrap()));  // 记 context→交集 rect
accumulated = intersected;
// 本节点 + 子树 mask_context = ctx
```
`rect_intersect`：标准 AABB 交集（max(x) , min(x+w)，无重叠→零面积）。

- [ ] **Step 4: blob.rs emit clip 表**

`build_blob` 接收 clip 表（从 batch 产出的 context→rect 映射，或 batch 直接产 `Vec<(u32,Rect)>`）。拼装 clip 表段：
```rust
clip_table_buf.extend_from_slice(&(clips.len() as u32).to_le_bytes());  // clip_count
for (ctx, r) in &clips {
    clip_table_buf.extend_from_slice(&ctx.to_le_bytes());
    clip_table_buf.extend_from_slice(&r.x.to_le_bytes());
    clip_table_buf.extend_from_slice(&r.y.to_le_bytes());
    clip_table_buf.extend_from_slice(&r.w.to_le_bytes());
    clip_table_buf.extend_from_slice(&r.h.to_le_bytes());
}
```
> 接口：`stage.tick_and_render()` 现 return `Vec<RenderNode>`；扩为也 return clip 表（或 build_blob 内部从 nodes 的 mask_context + 节点 clip_rect 重算——但交集需树遍历，batch 已算，宜 batch 产出）。**决策**：batch.rs DFS 时把 `(ctx, intersected_rect)` 存进一个 `Vec`，`tick_and_render` 一并 return（改签为 struct `{ nodes, clips }`），build_blob 消费。

- [ ] **Step 5: run，验绿**（嵌套不相交 → 交集零面积）

- [ ] **Step 6: commit**
```bash
git add loomgui_core/src/render/batch.rs loomgui_core/src/render/mod.rs loomgui_core/src/stage.rs loomgui_ffi_c/src/blob.rs loomgui_ffi_c/src/lib.rs
git commit -m "feat(v1a-p2): rect mask Rust — 嵌套 clip 交集 + clip 表 emit（修只裁最内层）"
```

---

### Task 6: rect mask Unity（_ClipBox per-context material 接线）

**Files:**
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs`
- Test: `Assets/LoomGUI/Tests/ClipBoxTests.cs`（EditMode）

**Interfaces:**
- Consumes: FrameBlob clip 表（T5）+ mask_context per node。
- Produces: mask_context>0 节点用 CLIPPED material + `_ClipBox`（design→world）SetVector。

**参考**：fgui `UpdateContext.cs:105-156`（_ClipBox = (-cx/hw,-cy/hh,1/hw,1/hh)）、`NGraphics.cs:604-644`（material 取 + ApplyClippingProperties）、`MaterialManager.cs`（group=clipId）。shader Phase 1 已有 CLIPPED variant。

- [ ] **Step 1: 失败测试（_ClipBox math）**

`ClipBoxTests.cs`：纯数学测——给 design rect {x=100,y=100,w=200,h=200} + 根 transform（scale=(1,-1,1), pos=(0,0)）→ `ComputeClipBox` 返回 `_ClipBox = (-cx/hw, -cy/hh, 1/hw, 1/hh)`。断言 center=(200,-200)（design 200 → world y=-200，x=200）、half=(100,100)、_ClipBox=(-2,2,0.01,0.01)（符号验）。
```csharp
[Test] public void ComputeClipBox_DesignRect_WorldCenterHalf() { ... }
```

- [ ] **Step 2: run，验红**

- [ ] **Step 3: MaterialManager — SetClipBox + CLIPPED keyword**

`MaterialManager.cs`：`Get` 创建 material 时若 mask_context>0 → `EnableKeyword("CLIPPED")`（variant）。加：
```csharp
public void SetClipBox(uint ctx, Vector4 clipBox) {
    // 找 (program,texture,ctx) 的 material，SetVector("_ClipBox", clipBox)
    // 若该 ctx material 未创建，先 Get 触发创建
}
```
> 关键：mask_context 已进 key（Phase 1），每个 ctx 一个 material 实例，`_ClipBox` SetVector 进该实例。Unity 用 `mat.SetVector`（fgui 同，非 MPB）。

- [ ] **Step 4: MirrorPool — clip 接线**

Sync 循环：transform 设置后，`uint mc = blob.MaskContext(i);` 取 material：`mm.Get(program, texture, mc)`。每帧首次见某 ctx（fgui firstMaterialInFrame）：算 `_ClipBox` 并 `mm.SetClipBox(mc, box)`。
```csharp
// design rect → world → _ClipBox（根 transform.TransformPoint 两角）
Vector2 wTL = root.TransformPoint(new Vector3(rect.x, rect.y));
Vector2 wBR = root.TransformPoint(new Vector3(rect.x+rect.w, rect.y+rect.h));
Vector2 center = (wTL+wBR)*0.5f, half = (wBR-wTL)*0.5f;  // half 取绝对
half.x = Mathf.Abs(half.x); half.y = Mathf.Abs(half.y);
Vector4 clipBox = new Vector4(-center.x/half.x, -center.y/half.y, 1f/half.x, 1f/half.y);
mm.SetClipBox(mc, clipBox);
```
> MirrorPool 维护 `HashSet<uint> _clipSetThisFrame`，每 ctx 首次 SetClipBox；帧开头清。半宽/高为 0 → safe-blank `(-2,-2,0,0)`（fgui 同）。

- [ ] **Step 5: run EditMode，验绿**

- [ ] **Step 6: PlayMode 视觉（手动，用户验）**

LoomStage `_html`/`_css`：overflow:hidden 容器 + 溢出子内容（如 `.clip{width:100px;height:100px;overflow:hidden} .big{width:300px;height:300px;background:blue}`）→ Play → 验蓝块被裁到 100×100。嵌套两层 overflow 验交集。用户报。

- [ ] **Step 7: commit**
```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs loomgui_unity/Assets/LoomGUI/Tests/ClipBoxTests.cs
git commit -m "feat(v1a-p2): rect mask Unity — _ClipBox（design→world）per-context material + CLIPPED 接线"
```

---

### Task 7: 500 节点静态压测

**Files:**
- Create: `loomgui_unity/Assets/LoomGUI/Tests/Stress500Fixture.cs`（或 LoomStage preset）
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`（buffer 复用，若卡顿）
- Test: 手动 PlayMode（无自动断言；frame-time 读数）

**Interfaces:**
- Consumes: 全管线（T1-T6）。
- Produces: 500 节点静态渲染无卡顿验收。

- [ ] **Step 1: 500 节点 fixture**

LoomStage 加 `[SerializeField] bool _stress500`；Awake 若 true → `_html`/`_css` 程序生成 500 节点（嵌套 div + 多 text）。或单独 stress 场景 preset。

- [ ] **Step 2: PlayMode + 帧时间读数**

Play（stress 模式）→ on-screen FPS（`OnGUI` 显示 1/Time.deltaTime）或 Profiler。**验收**：肉眼无卡顿（≥45fps 静态）。

- [ ] **Step 3: 若卡顿 → 最小 buffer 复用**

`MirrorPool.UploadMesh` 当前每帧每节点 new `Vector3[]/Color[]/int[]`。改：RenderObj 持可复用 `List<Vector3>/List<Color>/List<int>`，用 `Mesh.SetVertices(List)` overload（零 GC）。冷帧全量 ArrayPool 留 v1e。
```csharp
// RenderObj 加：List<Vector3> _vList; List<Color> _cList; List<int> _iList;
// UploadMesh 改：清 list → 填 → SetVertices(List) / SetColors(List) / SetTriangles(List)
```

- [ ] **Step 4: 再测，验无卡顿**

- [ ] **Step 5: commit**（若改了 UploadMesh）
```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs loomgui_unity/Assets/LoomGUI/Tests/Stress500Fixture.cs
git commit -m "perf(v1a-p2): 500节点压测——buffer 复用（若 naive alloc 卡顿）；全量 ArrayPool 留 v1e"
```
（若无卡顿、未改 UploadMesh，仅 commit fixture + ledger 记「500 静态无卡顿，buffer 复用留 v1e」。）

---

### Task 8: Domain reload 保护（接线 + 快进快出测）

**Files:**
- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs`
- Modify: `loomgui_ffi_c/src/lib.rs`（`loomgui_shutdown` 注释/保留）
- Test: 手动 PlayMode（禁 Domain Reload ×20）

**参考**：fgui `Stage.cs:86`（`[RuntimeInitializeOnLoadMethod(SubsystemRegistration)]`）。

- [ ] **Step 1: ResetStatics 接线**

`LoomStage.cs`：
```csharp
[RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]
static void ResetStatics() {
    Native.loomgui_shutdown();   // 核心 v1a 无全局态，近 no-op，但 hook 在
    // 清 C# 静态缓存（当前无 static；TextRasterizer.s_fontVersion 复位）
    TextRasterizer.ResetStatic();
}
```
`TextRasterizer` 加 `internal static void ResetStatic(){ s_fontVersion=0; }`。

- [ ] **Step 2: 手动测（禁 Domain Reload）**

Unity Editor → Project Settings → Editor → Enter Play Mode Options = ON, Reload Domain = OFF。快进快出 Play Mode ×20 → 无 crash、无 Console 野指针/泄漏报错。

- [ ] **Step 3: Font 泄漏观测**

×20 期间观察内存（Profiler Memory）有无无限增长（Font `Box::leak`）。若明显增长 → ledger 记 + 评估 Phase 2 内做字体缓存化（进程单例）；否则记「无显著增长」。

- [ ] **Step 4: commit**
```bash
git add loomgui_unity/Assets/LoomGUI/Runtime/LoomStage.cs loomgui_unity/Assets/LoomGUI/Runtime/TextRasterizer.cs
git commit -m "feat(v1a-p2): Domain reload 保护——ResetStatics 接 loomgui_shutdown + 清静态；禁 reload ×20 不 crash"
```

---

## 实现后（全 task 完）

- final whole-branch review（superpowers:requesting-code-review，opus/最能干模型，全分支 diff）。
- `superpowers:finishing-a-development-branch`（合并 main / PR / cleanup）。
- session-summary skill 总结 Phase 2 经验进 knowledge-reference（text 光栅坑/clipBox/flatten 坐标 bug/Unity 6.5 API）。
