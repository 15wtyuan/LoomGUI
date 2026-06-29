# v1.1 background-image 共存视觉修复（方案 A）

> **状态**：待实现。3.6/3.7"内容空"已修（见 §1），但 3.6 background-image 与 background-color 共存视觉仍不对——图透明区透 root 不透 background-color。本文档为明天继续的备忘。
> **日期**：2026-06-30。

---

## 1. 已修复：3.6/3.7 内容空（CLIPPED + scroll）

### 根因
`.bg-demo`/`.br-demo` 的 `overflow:hidden` 触发 CLIPPED（`mask_context>0`）。clip rect 在 `batch.rs` 按 **design 空间**（layout 绝对）算交集，但 scroll 容器的子节点 world_matrix 含 `T(-scroll_pos)`（`transform.rs:45`）——shader `clipPos = worldPos.xy × _ClipBox.zw + _ClipBox.xy` 用的是 **world 空间（含 scroll）**，而 `_ClipBox` 由 `ComputeClipBox(root, design_rect)` 算（**不含 scroll**）。

后果：bg-demo layout.y=4337，main-scroll viewport y∈[138,1920]，design 空间本不相交 → 交集 **h=0** → `ComputeClipBox` 返 SafeBlank `(-2,-2,0,0)` → shader `clipPos` 恒 `(-2,-2)` → `step(max|clipPos|,1)=step(2,1)=0` → **col.a=0 完全透明（全裁）**。

scroll 越深越明显（用户滚到 §3.7，scroll 几千 px）。3.1–3.5 无 `overflow:hidden` → 不 CLIPPED → 正常显示，所以看起来"只有 3.6/3.7 坏"。

### 修复（commit `e2b64bd` + dll `66387aa`）
`batch.rs::assign_sort_keys` 的 dfs 加 `scroll_offset: (f32,f32)` 累积参数：
- **own clip 减 scroll_offset**（本节点 world 在 `layout - scroll_offset` 空间，clip rect 同空间）。
- **accumulated 不减 scroll**（祖先 clip 如 scroll 容器 viewport 在 world **固定**——容器自身 world 不含自己 scroll_pos）。
- `intersected = accumulated(design) ∩ own(world)` = **world 可见区**。

### 踩过的坑（v1 错修，a34c972 已被 v2 取代）
v1 把 accumulated 也减 scroll_delta——错：viewport 跟着移动，与 own 求交仍 h=0（design 不相交，同减 scroll 相对位置不变）。v2 才对。

### 另一个坑：`set_scroll_pos` 时机
`set_scroll_pos` 用 `scene.scroll.get_mut(node)`，但 scroll 表在 **tick 时**才 ensure（overflow:scroll 节点入表）。**tick 前调 set_scroll_pos → scroll 表空 → no-op**。诊断/测试时必须先 tick 一次建表，再 set_scroll_pos，再 tick。PlayMode 里用户拖动滚动时表已建，不受影响。

---

## 2. 待办：方案 A——background-image 共存视觉

### 问题
3.6 现在显示了，但 home.png（透明背景 icon）的透明区透出 root 深蓝，**透不出 background-color**（青/红底看不见）。

shader `LoomGUI-Unlit` program:0 frag：
```hlsl
half4 col = tex * vcol;   // = (tex.rgb*vcol.rgb, tex.a*vcol.a)
```
`tex.a=0`（图透明区）→ `col.a=0` 完全透明。这是简单 tint，**不是 CSS background 合成**（图透明区应透 background-color）。违背 v1.1 设计 §6.2 承诺。

### 关键约束
**img 节点和 Container+background-image 节点都用 tex1（atlas 纹理）**，shader 无法靠 texture 区分。当前 MirrorPool 按 `payload_kind` 硬编码 program（Mesh=0, Text=1），**program 不进 frame blob**。

→ 必须用 **program 号**区分（img=0 保持 tex×vcol；Container+bg-image=2 走合成），所以 **program 得进 frame blob**。这是方案 A 最大的改动，其余都是跟着接。

### 改动面（5 处，TDD）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `loomgui_ffi_c/src/blob.rs` + `FrameBlob.cs` + blob TestView | **加 program 列**：columns 末尾加 `("program",1)` + `col_program`（写 mesh/text 的 program）；**VERSION 4→5**。FrameBlob `ColOff(18)` + `Program(i)` 方法。TestView 同步。TDD：program round-trip（写 2 读 2）。 |
| 2 | `LoomGUI-Unlit.shader` | 加 `#pragma multi_compile _ BG_COMPOSITE`；frag 分支：`#if defined(BG_COMPOSITE) col.rgb=tex.rgb*tex.a+vcol.rgb*(1-tex.a); col.a=vcol.a; #else col=tex*vcol; #endif` |
| 3 | `loomgui_core/src/render/mod.rs` | Container/Button 分支：有 `background_image` 且纹理命中 → `program=2`；无 bg-image → `program=0`（保持）。Image 节点保持 `program=0`。 |
| 4 | `MaterialManager.cs` | `Get`：`if (program==2) mat.EnableKeyword("BG_COMPOSITE");` |
| 5 | `MirrorPool.cs` | Mesh 路径 `mm.Get(program: 0, …)` → `mm.Get(blob.Program(i), …)`（不硬编码）。 |

### 合成公式（CSS background：图在色块上）
```
col.rgb = tex.rgb * tex.a + vcol.rgb * (1 - tex.a)   // 不透明区显图，透明区显 bg-color
col.a   = vcol.a                                       // 整体 alpha 由 bg-color 决定
```
- img 节点（program:0，vcol=白）：`tex*白=tex`，图透明区透明透下层——保持不变。
- Container+bg-image（program:2，vcol=bg-color）：图透明区透 bg-color——兑现设计 §6.2。

### 验收
重启 PlayMode，§3.6：
- 第 2 行（`bg-color:#5fb2c4` 青 + home.png contain）：图外区显**青底**，icon 显图。
- 第 3 行（红 + 100%）：红底 + 拉伸图。
- 第 1 行（深蓝 + cover，bg-color=root）：深蓝底 + 图（和 root 同色，主要看 icon）。

### 实现顺序建议
1. step 1（blob program 列 + round-trip 测试）——地基，VERSION bump。
2. step 3（core Container+bg-image program=2）+ core 测试。
3. step 2/4/5（shader + MaterialManager + MirrorPool）一起，Unity 侧。
4. 重编 .dll + cp + PlayMode 验收。

---

## 3. 涉及文件清单

- `loomgui_ffi_c/src/blob.rs`（+ TestView）—— program 列、VERSION 5
- `loomgui_unity/Assets/LoomGUI/Runtime/FrameBlob.cs`—— ColOff(18)、Program(i)
- `loomgui_unity/Assets/LoomGUI/Shaders/LoomGUI-Unlit.shader`—— BG_COMPOSITE
- `loomgui_core/src/render/mod.rs`—— Container 分支 program=2
- `loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs`—— program==2 keyword
- `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`—— 用 blob.Program(i)
- `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`—— 重编

**零改**：pkg.bin 格式（v9 不变，program 是 frame blob 字段，不是 scene 字段）、img 节点路径、text 路径。
