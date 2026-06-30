# v1.1 background-image 共存视觉修复（坑 79）— 设计 spec

> **日期**：2026-06-30。
> **背景**：v1.1 §6.2 承诺"Container 设 background-image 时，图透明区显 background-color"，但 shader `tex*vcol` 是简单 tint——图透明区 `col.a=0` 全透明，透不出 bg-color。本 spec 固化修复设计。
> **关联 plan**：`docs/superpowers/plans/2026-06-30-bg-image-composite.md`（已核对准确，本 spec 是其设计层固化）。

---

## 1. 问题

§3.6（v1-showcase）Container 设 `background-color:#5fb2c4` + `background-image:home.png`（透明背景 icon）。实机：图透明区透出 root 深蓝，**透不出青底 bg-color**。

shader `LoomGUI-Unlit` frag（program:0）：
```hlsl
half4 col = tex * vcol;   // (tex.rgb*vcol.rgb, tex.a*vcol.a)
```
`tex.a=0`（图透明区）→ `col.a=0` 完全透明。这是 tint，**不是 CSS background 合成**。

## 2. 关键约束

**img 节点和 Container+background-image 节点都用 tex1（atlas 纹理）**，shader 无法靠 texture 区分。当前 MirrorPool 按 `payload_kind` 硬编码 program（Mesh=0, Text=1）。

→ 必须用 **program 号**区分：img=0 保持 `tex×vcol`；Container+bg-image=2 走合成。所以 **program 得进 frame blob**（当前 payload 里有 program 字段，但 FFI blob 没序列化，MirrorPool 只能硬编码 0）。

## 3. 设计决策

### 3.1 program 号语义
| program | 节点 | frag | vcol |
|---|---|---|---|
| 0 | Image / 无 bg-image 的 Container | `col = tex * vcol` | img=白；Container=bg-color（tex=白占位 → 显 bg-color） |
| 1 | Text | `tex.a * vcol`（ALPHA_MASK keyword） | text color |
| 2 | **Container+bg-image**（新增） | CSS 合成（见 3.2） | bg-color |

**Container 分支 program 取值**（mod.rs）：
- 有 `background_image` 且纹理命中（`textures.get(url).is_some()`）→ `program=2`
- 否则（无图 / 未注册哨兵）→ `program=0`（保持 `tex*vcol`，白占位×bg-color=bg-color）

### 3.2 CSS 合成公式（图在色块上）
```
col.rgb = tex.rgb * tex.a + vcol.rgb * (1 - tex.a)   // 不透明区显图，透明区显 bg-color
col.a   = vcol.a                                       // 整体 alpha 由 bg-color 决定
```
- img（program:0，vcol=白）：`tex*白=tex`，图透明区透下层——保持不变。
- Container+bg-image（program:2，vcol=bg-color）：图透明区显 bg-color——兑现 §6.2。

### 3.3 program 列类型：u8
与 `payload_kind`/`visible` 同类（1 字节），值域 0/1/2 够用。blob 列 18→19，VERSION 4→5。

### 3.4 圆角 + bg-image 共存（核对澄清）
Container 既有 bg-image 又有 border-radius（§3.7 R5）时：rounded_rect 三角扇**只覆盖圆角矩形内部**，角上镂空区无顶点/三角形 → 无 fragment → 天然透明透下层。合成只发生在圆角矩形内部 fragment 上。**合成公式对圆角+bg-image 共存安全，不需特判**。

### 3.5 现状：program 字段已存在（v1.2 半成品地基）
`NodePayload::Mesh { ..., program }` 字段已就位（Container/Image 硬写 0，Text 硬写 1）。`MaterialManager.Get(program, tex, ctx, matrixFlag)` 已按 program keying。**核心 gap 仅在 FFI 序列化层**：payload 有 program，blob 没序列化，MirrorPool 硬编码 0。

## 4. 改动面（5 处，TDD）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `blob.rs` + `FrameBlob.cs` + blob TestView | 加 program 列（**u8**，第 19 列）+ `col_program`；**VERSION 4→5**。FrameBlob `ColOff(18)` + `Program(i)`。TestView `col_off[18]→[19]`。TDD：program round-trip（写 2 读 2）。 |
| 2 | `LoomGUI-Unlit.shader` | `#pragma multi_compile _ BG_COMPOSITE`；frag 分支：`#if defined(BG_COMPOSITE) col.rgb=tex.rgb*tex.a+vcol.rgb*(1-tex.a); col.a=vcol.a; #else col=tex*vcol; #endif` |
| 3 | `render/mod.rs` | Container 分支：有 bg-image 且纹理命中 → `program=2`；否则 `program=0`。Image 保持 `program=0`。（只改赋值，字段已存在） |
| 4 | `MaterialManager.cs` | `Get`：`if (program==2) mat.EnableKeyword("BG_COMPOSITE");` |
| 5 | `MirrorPool.cs` | Mesh 路径 `mm.Get(program: 0, …)` → `mm.Get((int)blob.Program(i), …)`（不硬编码） |

## 5. 零改

- pkg.bin 格式（v9 不变——program 是 frame blob 字段，不是 scene/ResolvedStyle 字段）。
- img 节点路径（program:0 不变）。
- text 路径（program:1 不变）。

## 6. 验收

重启 PlayMode，§3.6：
- 第 2 行（青 + home.png contain）：图外区显**青底**，icon 显图。
- 第 3 行（红 + 100%）：红底 + 拉伸图。
- 第 1 行（深蓝 + cover，bg-color=root）：深蓝底 + 图（和 root 同色，主要看 icon）。
- §3.7 R5（图+圆角共存）回归：圆角镂空区透下层，圆角内图透明区显 bg-color。

## 7. 实现顺序

1. step 1（blob program 列 u8 + round-trip 测试）——地基，VERSION bump。
2. step 3（core Container+bg-image program=2）+ core 测试。
3. step 2/4/5（shader + MaterialManager + MirrorPool）一起，Unity 侧。
4. 重编 .dll + PlayMode 验收（家里机）。
