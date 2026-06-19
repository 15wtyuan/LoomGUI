# LoomGUI v1a Phase 2 · 渲染补全（Text + rect mask + 压测 + Domain reload）设计（spec）

- 日期：2026-06-19
- 状态：待审 → 批准后进 writing-plans
- 依据：`docs/design/00-main-design.md`（v1 实现真相源）、`docs/roadmap/v1-scope.md`、`docs/superpowers/specs/2026-06-19-v1a-unity-render-design.md`（v1a Phase 1 spec）
- 参考实现：FairyGUI-unity（`temp/FairyGUI-unity/`，文本光栅/clip 的机制参考）
- 产出方式：superpowers brainstorming 流程

---

## 1. 目的

v1a Phase 1 只打通了**最薄渲染竖切**（单 quad：FFI blob + 镜像池 + URP shader + 根 Stage，PlayMode 红块渲染成功）。v1a spec §4.4 钉的 7 步实现顺序里，Phase 1 只做了前 2 步半。

**Phase 2 = 把 v1a 关到它自己的验收门**（v1a spec §9）：补完 rect mask、Text、500 节点压测、Domain reload 四项，让 v0 fixture（div flex + 文本 + rect mask）在 Unity 真渲出来、500 节点静态无卡顿、进出 Play 不 crash。

> 命名说明：拆分表里 `v1b` = 打包器+资源管线。本 spec 不是 v1b，是 **v1a Phase 2（渲染补全）**——v1a 自己未做完的范围。打包器仍是之后的 v1b。

---

## 2. 范围边界

**做**：
- **Text 渲染**（最高风险）：Rust 把 TextLayout 序列化进 blob text_arena；Unity 当纯光栅器（`RequestCharactersInTexture` + `GetCharacterInfo` 取 UV/box，笔位严格按 Rust）+ `Font.textureRebuilt` 监听。ASCII 单字体（DejaVu）。
- **rect mask**：Rust emit clip 表（context_id → 绝对 design rect，**嵌套取交集**）；Unity 算 `_ClipBox`（design→world）挂 per-context material，shader CLIPPED variant discard。
- **多节点 + `sort_key`→`sortingOrder` PlayMode 视觉验证**（v1a §4.4 step 3，Phase 1 只 EditMode 测过 diff 逻辑）。**顺带修一个 latent 坐标 bug**（见 §4.2）。
- **500 节点静态压测**（便宜帧，无卡顿；冷帧/换页帧 ≤2ms 留 v1e）。
- **Domain reload 保护**：`loomgui_shutdown()` 接线 + C# `ResetStatics` 清静态 + 禁 Domain Reload 快进快出 Play ×N 不 crash。
- Blob VERSION 1→2 + magic/version 校验。

**不做**（见 §7 defer 表）：
- CJK / 多字体 / fallback 链 / 字体资源完整化 → v1b（打包器同批）。
- 文本测量 cache `(text,font,size,constraint)→(w,h)` → v1e（naive 重算够 500 节点）。
- 冷帧/换页帧 FFI ≤2ms（ArrayPool）、FairyBatching 实机优化 → v1e。
- grayed keyword（shader 预留即可，不接线）、SRP Batcher/合 mesh、shape mask/stencil、soft clip → v1.x。
- 行内富文本/选中/光标/IME（TextInput 围栏外）。

**范围决策记录**（brainstorming 钉定）：
- **不拆 2a/2b，一个 Phase 2 全包**：rect mask + Text + 压测 + Domain reload 一个 spec/plan。Text 是大头但与其余三项共用 blob/镜像池基建，分开反而割裂。
- **嵌套 clip 在 Phase 2 就修对**（fgui 是交集；v0 当前只裁最内层是错的），不 deferred。
- **Text 偏离 fgui 的 advance/行高**（用 Rust ttf 真度量），照搬 fgui 的 UV/box/textureRebuild/clipBox 机制。

---

## 3. 上下文（v1a Phase 2，地基已通）

Phase 1 已闭合最大风险缝：Rust→FFI→Unity→URP 单 quad 真渲。Phase 2 在此基础上**只补内容**，不重做基建。

v0 核心已就绪（Phase 2 不重写算法，只加序列化 + 接线）：
- `TextLayout`（lines/runs/glyphs 三表，ttf-parser 真度量，绝对 glyph 坐标）— `loomgui_core/src/text/layout.rs:13-52`。
- `NodePayload::Text { layout, font_size, color, program }` 已 emit — `render/mod.rs:92`。
- `overflow:hidden` → `clip_rect`（绝对 design 坐标）由 layout 填 — `layout/mod.rs:175`。
- `mask_context` 层 id 由 batch DFS 分配 — `render/batch.rs`。
- `loomgui_shutdown()` 空壳 — `ffi_c/src/lib.rs:135`。

**Phase 2 新工作 = FFI 序列化 + Unity 接线**（核心算法 0 重写）：
1. text_arena 序列化（Rust emit）+ Unity 光栅消费。
2. clip 表 emit（Rust，含嵌套交集）+ Unity `_ClipBox` 接线。
3. 多节点坐标 bug 修。
4. 压测 fixture + Domain reload 接线。

---

## 4. 设计

### 4.1 Blob v2 扩展（FFI 命门）

在 Phase 1 单块 blob（mesh_arena）上加 **text_arena 段 + clip 表段 + 2 个 per-node 列**，VERSION 1→2。保持 §14.3「单块原子拷贝」。

**Header v2（88B）**：
```
magic(u32)=0x4D4F4F4C | version(u32)=2 | node_count(u32)
13× col_offset(u32)                              // 比 v1 多 text_off/text_len
mesh_arena_off(u32) | mesh_arena_len(u32)
text_arena_off(u32) | text_arena_len(u32)        // 新
clip_table_off(u32) | clip_table_len(u32)        // 新
```

**Per-node 13 列**（v1 的 11 列 + `text_off`(u32) + `text_len`(u32)）：
```
node_id | parent_id | visible | alpha | sort_key | local_x | local_y | mask_context
| payload_kind(0=Unchanged/1=Mesh/2=Text) | mesh_off | mesh_len | text_off | text_len
```

**text_arena 段**（每 text 节点，位于 `text_arena_off + node.text_off`）：
```
font_size(u32) | color(f32×4) | glyph_count(u32)
glyphs[glyph_count × { codepoint(u32), pen_x(f32), pen_y(f32) }]   // 12B/glyph
```
- `codepoint`：Unicode 码点（**Unity `GetCharacterInfo(char)` 要码点，不是 glyph_id**）。Rust `Glyph` 结构当前只存 ttf `glyph_id`，Phase 2 加 `codepoint` 字段（`measure_text` 本就遍历 char，顺带存）。
- `pen_x/pen_y`：**绝对 design 坐标、且已含 content-box 偏移、相对节点 GO 原点**（= layout_rect 原点）。Unity 直接摆，**不 re-base**（mesh 是 re-base 的，因为 v0 mesh 顶点是绝对；text layout 本就是节点局部，content 偏移在 Rust 烤进 pen）。pen_y = line.y + line.baseline（行信息是 measure 副产物，Phase 2 不单送 line 表；多字体/CJK 回来再补 runs/lines）。
- `color` = text 前景色（= style.color，即 v0 坑 9 里那个被误用的 color_tint；text 用它**是对的**）。shader 顶点色 = color × node alpha。

**clip 表段**（位于 `clip_table_off`）：
```
clip_count(u32)
entries[clip_count × { context_id(u32), x(f32), y(f32), w(f32), h(f32) }]   // 20B/entry
```
- rect = **绝对 design 坐标**（layout 已算绝对；见 §4.2），**嵌套已取交集**（§4.4）。
- `mask_context==0`（无 clip）不入表；只入 context>0。

**所有权/拷贝**不变：Rust 拥有 per-frame blob，下帧 tick reset；C# borrow 后 `Marshal.Copy` 到托管 buffer 一次，只读自身拷贝。

### 4.2 坐标契约（Phase 2 必修 latent bug）

**事实**：layout `write_back` 递归累加 parent origin（`layout/mod.rs:160-184`）→ **`local_x/local_y` 与 `clip_rect` 都是绝对 design 坐标**（非父相对）。根 GO 映射 design→world：`localScale=(sf,-sf,sf)`、`localPosition=(-sw/2, sh/2)`，design (dx,dy) → world (-sw/2+dx·sf, sh/2−dy·sf)。

**Latent bug（Phase 1 未暴露）**：`MirrorPool.cs:49-54` 按 `parent_id` **巢状** SetParent，但 `localPosition=(local_x,local_y)` 是**绝对**。巢状下子节点 world = root(父abs) + (子abs) → **父被双重计入**，多节点位置错。Phase 1 默认场景单节点（pid=−1→parent=root）所以没炸。

**Phase 2 修法（推荐 flatten）**：所有节点 `SetParent(root)`（置 `parent_id` 于不顾），`localPosition=绝对`。world = root(绝对) = 正确，且与 clip（绝对 design→root transform）一致。mesh 顶点已 re-base 到节点局部（减 transform.x/y），GO 在绝对位 + 局部顶点偏移 → world = root(绝对+偏移) = 正确。
- `parent_id` 仍在 blob 保留（v1c 事件传播 / 未来 transform 继承要用），Phase 2 渲染不用。
- 备选（若更早要 GO 层级）：巢状但 `localPosition = 子abs − 父abs`（转父相对）。Phase 2 取 flatten（最简、正确、YAGNI）。
- **clip 一致性**：clip rect 绝对 design → Unity 用**根 transform** 的 `TransformPoint` 映射两角到 world（非逐 clipper 矩阵，因为 clip 是绝对 design 不是 clipper-local）。

### 4.3 Text 渲染契约（最高风险，钉死）

**原则：Rust 是布局唯一权威，Unity 是纯光栅器。**

每个 text 节点 → 一个 Mesh（所有 glyph quad 累加），texture = 动态字体 atlas（`Font.material.mainTexture`），program=1（text，与 Image 共用 `LoomGUI/Unlit` shader）。

**Unity 每 text 节点**：
1. 读 text_arena：font_size、color、glyphs[]。
2. 收集所有 codepoint 成 string → `font.RequestCharactersInTexture(str, font_size, FontStyle.Normal)`（填 atlas）。
3. 每 glyph：`font.GetCharacterInfo((char)codepoint, out info, font_size, Normal)` 取 **UV 四角 + 像素 box（minX/maxY/maxX/minY）**。
4. quad（y-down design，GO-local，**不 re-base**）：
   - 笔位 = (pen_x, pen_y)（Rust 绝对权威）。
   - box 偏移/尺寸 = Unity 的 (minX,maxY,maxX,minY)（光栅器权威）：
     - quad_left = pen_x + minX，quad_right = pen_x + maxX
     - quad_top = pen_y − maxY，quad_bottom = pen_y − minY（maxY 在基线上方→y-down 减）
   - UV = info.uvBottomLeft/TopLeft/TopRight/BottomRight 映射四角。
   - 色 = color × node_alpha（四顶点同）。
5. **不用** Unity `CharacterInfo.advance`（笔位用 Rust）；**不用** Unity `fontSize*1.25` 行高（pen_y 已含 Rust 行布局）。

**`Font.textureRebuilt` 监听（必修坑）**：动态字体 atlas 异步 rebuild 时 **glyph UV 会变**。注册 `Font.textureRebuilt += cb`；cb 置 dirty flag + version++；下帧 dirty → 重新 `RequestCharactersInTexture` + 重取 UV（照 fgui `DynamicFont.cs:356-375` + `Stage.cs:828` 重跑帧）。不抄会画错字/花字。

> baseline/box 的正负号具体数学在 plan 钉 + EditMode 测锁（glyph 笔位 == Rust layout 断言）。

### 4.4 rect mask（`_ClipBox`，照搬 fgui）

**Rust 侧**：
- batch DFS 算**嵌套交集**：维护 `accumulated_clip: Option<Rect>`（祖先 clip 链的交）；遇 overflow:hidden 节点，新 context 的 rect = `accumulated_clip ∩ 本节点 clip_rect`，更新 accumulated。修当前「只取最内层」（v0 batch 只赋新 context 不交，嵌套不相交 clip 会漏裁）。
- emit clip 表（§4.1）：context>0 → 交集后的绝对 design rect。

**Unity 侧**（MirrorPool 接线，当前 kind=2 跳过、SetClipBox 未测）：
- 每 node 读 `mask_context`；>0 则用 CLIPPED material：`mm.Get(program, texture, mask_context)`（mask_context 已进 key，Phase 1 做了）。
- 每 context 每帧首次（fgui `firstMaterialInFrame` 模式）算 `_ClipBox` 并 SetVector：
  - design rect 两角 → world（根 transform `TransformPoint`）→ world center/half。
  - `_ClipBox = (-cx/halfW, -cy/halfH, 1/halfW, 1/halfH)`（照 fgui `UpdateContext.cs:105-156`）。
  - shader（Phase 1 已有 CLIPPED variant）：`clipPos = TransformObjectToWorld(pos).xy × _ClipBox.zw + _ClipBox.xy`；`col.a *= step(max(abs(clipPos)),1)`。
- 嵌套由 Rust 交集折叠成单 box/单 context（fgui 同），Unity 每 context 一个 `_ClipBox`。

### 4.5 500 节点静态压测

- **fixture**：HTML 生成 ~500 节点（嵌套 div + 多个 text/img-placeholder），Rust 已能产。
- **Unity**：PlayMode 渲染 + 帧时间读数（on-screen FPS 或 Profiler）。
- **验收**：静态 500 节点肉眼无卡顿（v1a §9.3 便宜帧）。冷帧/换页帧 ≤2ms 留 v1e。
- **已知 GC 风险**：`MirrorPool.UploadMesh` 每帧每节点 new `Vector3[]/Color[]/int[]`（Phase 1 review Minor）。500 节点 × 每帧可能卡。**若压测卡顿**：Phase 2 做最小 buffer 复用（每 RenderObj 持可复用 List，用 `SetVertices(List)` overload）；全量 `ArrayPool` 冷帧零 GC 留 v1e。

### 4.6 Domain reload 保护（G13，照 fgui `Stage.cs:86`）

- C# `LoomStage.ResetStatics`（当前空壳）→ 调 `Native.loomgui_shutdown()` + 清 C# 静态缓存（当前 MaterialManager/MirrorPool 都是 per-instance，随 MonoBehaviour OnDestroy 销毁；无 static，但 hook 必须在 + 调 FFI）。
- Rust `loomgui_shutdown()`：核心当前无全局态（Stage per-handle，随 `stage_free` drop）。Phase 2 保持近 no-op 但**接线存在**；若 Phase 2 引入任何全局 native 态（预计无），shutdown 清之。
- **测**：Unity 关 Domain Reload（Editor 设置），快进快出 Play Mode ×N（如 20），无 crash、无 Console 野指针报错。
- **已知 Font 泄漏**：`Font::from_bytes` 用 `Box::leak` 取 'static（v0 简化），每次 Stage 创建泄漏一份字体字节。Domain reload 测时验内存有无无限增长；若明显增长，Phase 2 做字体缓存化（进程内单例复用），否则记 ledger。

### 4.7 实现顺序（风险隔离，照 v1a §4.4 续）

8. **Blob v2 scaffold**：加 text_off/text_len 列 + text_arena/clip 表 header + version=2 + magic/version 校验。Rust emit 空 text/clip，C# 解析 header。验：现有 mesh 仍跑通、version==2、round-trip。
9. **多节点坐标修 + sort_key 视觉**（§4.2 flatten）：多节点 PlayMode 视觉验绘制序 + 位置正确。
10. **Text Rust emit**：`Glyph`+codepoint；序列化 TextLayout 进 text_arena。Rust TestView round-trip 测。
11. **Text Unity 光栅**：RequestCharacters/GetCharacterInfo/textureRebuild。EditMode mesh-gen 测（笔位==Rust）+ PlayMode 视觉（ASCII 文本渲出）。
12. **rect mask Rust**：嵌套交集 + clip 表 emit。Rust 测（嵌套不相交 → 交集 rect）。
13. **rect mask Unity**：clip 表 → `_ClipBox` → per-context material。EditMode clip math 测 + PlayMode 视觉（overflow:hidden 裁住溢出）。
14. **500 节点压测**：fixture + 帧时间；若卡顿做最小 buffer 复用。
15. **Domain reload**：接线 + 快进快出 ×N 测。

---

## 5. fgui 对照表（★「实现前先参考 fgui」准则）

| 机制 | fgui | LoomGUI Phase 2 | 处置 |
|---|---|---|---|
| glyph UV + 像素 box | `GetCharacterInfo` minX/maxY/maxX/minY + uv 四角 | 同 | **照搬** |
| glyph 笔位 x | 信 Unity `CharacterInfo.advance` | **Rust ttf 真 advance** | **故意偏离（§9.1 跨平台一致性根）** |
| 行高 | `_font.fontSize*1.25` | **Rust TextLayout 已烤** | **故意偏离** |
| atlas rebuild | `Font.textureRebuilt`→flag+version，Stage 重跑帧（DynamicFont.cs:356-375 / Stage.cs:828） | 同 | **照搬（必修坑：UV 变）** |
| `_ClipBox` 公式 | `(-cx/halfW,-cy/halfH,1/halfW,1/halfH)`，clipPos=worldPos·zw+xy（UpdateContext.cs:105-156） | 同 | **照搬** |
| clip 坐标空间 | 世界空间（clipper local→world，逐 clipper TransformRect） | design 绝对 emit，Unity 走**根** transform | **同语义**（clip 是绝对 design，非 clipper-local，故用根而非逐 clipper） |
| 嵌套 clip | **交集折叠成单 box** | Rust DFS 算交集（修 v0「只取最内层」） | **对齐 fgui** |
| material key 含 clipId | `(flags, blend, group=clipId)`，`_ClipBox` SetVector 进 material | 同（mask_context 进 key，Phase 1 已做） | **照搬** |
| tint/alpha | 烤顶点色 | 同（text 顶点色=color×alpha） | **照搬** |

---

## 6. 主文档订正（本 spec 引入）

写本 spec 时对照 fgui + 读源码，需确认/订正 `docs/design/00-main-design.md`：
- **嵌套 clip 行为**：§8.6（rect mask）需明确「嵌套 overflow:hidden 取**交集**」（fgui 同）。若 §8.6 当前未写嵌套行为，补一行。Phase 2 实现按交集（fgui-faithful）。
- **坐标模型**：§14.x 渲染树契约补「节点 local_x/local_y 与 clip_rect 均为**绝对 design 坐标**；后端 flatten 到根 GO（Phase 2），nesting+父相对留待 transform 继承需求（v1c+）」。属实现契约澄清。
- text_arena/clip 表 blob 布局属 §14.3 FFI 细节，本 spec 已定，主文档 §14.3 的 text_arena 占位可对齐。

> 订正动作：spec 批准后、进 plan 前，按上 3 条最小改 00-main-design.md（设计契约才进 docs；实操/踩坑进 skill）。

---

## 7. defer → 落地表

| defer 项 | 落地阶段 | 依据 |
|---|---|---|
| CJK / 多字体 / fallback 链 / 字体资源完整化 | v1b（G6） | v1-scope §3；v1a spec §4.2d |
| 文本测量 cache `(text,font,size,constraint)→(w,h)` | v1e | v1-scope §7 |
| 冷帧/换页帧 FFI ≤2ms（ArrayPool）+ FairyBatching 实机 | v1e | v1a spec §7 |
| grayed keyword 接线、SRP Batcher/合 mesh、shape mask/stencil、soft clip、scale9/tile/fill | v1.x | v1x-deferred.md |
| Font `Box::leak` 缓存化（若 Domain reload 测出增长） | Phase 2 内或 v1e | §4.6 |

每条有归属，不悬空。

---

## 8. 风险（高→低）

1. **Text glyph box/baseline 正负号数学**（§4.3 step 4）— 偏一点就整行错位。缓解：EditMode 测断言 glyph 笔位/quad == Rust layout；PlayMode 肉眼校文本基线。
2. **`Font.textureRebuilt` UV 变**（§4.3）— 不监听就画花字。缓解：照搬 fgui 回调+version+重跑。
3. **多节点坐标 bug 修**（§4.2 flatten）— 改不好全盘位置错。缓解：多节点 PlayMode 视觉为先；flatten 最简且与 clip 一致。
4. **clipBox 坐标空间**（design→world，§4.4）— 算错就裁错/不裁。缓解：EditMode clip math 测 + fgui 公式照搬。
5. **嵌套 clip 交集正确性**（§4.4）— 算错漏裁。缓解：Rust 测嵌套不相交 → 交集空。
6. **500 节点 GC 卡顿**（§4.5）— naive 每帧 alloc。缓解：压测先跑，卡了做最小 buffer 复用。
7. **Domain reload 野指针/Font 泄漏增长**（§4.6）。缓解：禁 reload ×N 测；泄漏增长则缓存化。

---

## 9. 验收标准（"Phase 2 跑通"）

1. **Text**：ASCII 文本（DejaVu）在 Unity Play 真渲，位置/基线与 Rust TextLayout 一致（EditMode 笔位断言 + PlayMode 肉眼）。
2. **rect mask**：`overflow:hidden` 容器裁住溢出子内容；嵌套 clip 取交集正确（单层 + 嵌套不相交 PlayMode 视觉）。
3. **多节点 + 绘制序**：多节点位置正确（flatten 修复）、`sort_key`→`sortingOrder` 绘制序对（PlayMode 视觉）。
4. **500 节点静态无卡顿**（便宜帧；帧时间读数）。
5. **Domain reload**：禁 Domain Reload 快进快出 Play ×N（≥20）不 crash、无野指针。
6. **Blob v2**：magic+version 校验生效；text_arena + clip 表 Rust↔C# round-trip（单测）。

---

## 10. 工期估算（参考）

~3-4 周。分阶段：
- Blob v2 scaffold + 多节点坐标修：~0.5-1 周
- Text（Rust emit + Unity 光栅 + textureRebuild）：~1.5 周（最高风险）
- rect mask（Rust 交集 + Unity _ClipBox）：~0.5-1 周
- 500 压测 + Domain reload + 打磨：~0.5-1 周
