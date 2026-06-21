# v1b.4 — Mesh 合并 + AABB 保序重排 设计

> 设计原则：**简单优雅，不过度设计**。该做的做透，不该做的一个不加（YAGNI 清单见 §12）。

## 1. 目标 + 范围

**目标**：真 N→1 draw call —— 同 batch 的多 quad 在 core 拼成单个 mesh → 1 draw call。

**范围**：core 端做两件事——① AABB 保序重排 ② 显式 mesh 合并。**blob v3 / FrameBlob / MaterialManager / MirrorPool 零改**；Unity 仅因节点变少自然降 draw call + shader 保 SRP-Batcher compatible（免费附加）。

**不在范围**：Text 合并（mesh 在后端生成）、动画 opt-out、增量 dirty、GPU instancing（见 §12）。

## 2. 现状认知

- `§8.5` 当前「不合并 mesh（每节点各自原生渲染对象）」，`batch.rs` v0 简化：`sort_key = DFS 出现序`，注释明说「不做 AABB 重排合并……留待 v1.x」。
- 每节点独立 `GameObject+MeshRenderer` → draw call = N。v1b.3 图集只给**同 Material**（SRP Batcher CPU 效率），**没降 draw call 数**。
- **调研关键发现**：fgui `DoFairyBatching`（`Container.cs:877-941`）**也不合并 mesh** —— 每元素独立 `NGraphics`，只重排 `sortingOrder` 让同 material 相邻，靠 **Unity Dynamic Batching 隐式合并**（不可控、URP 下与 SRP Batcher 互斥）。LoomGUI 要 core **显式合并**，补 fgui 没做、确定性、跨后端的一步。

## 3. 调研结论（为何不另起路线）

fgui 算法核心（稳定排序 + AABB 保序重排）是 quad-UI 合批最优通用解；**无更优现代替代**：
- SRP Batcher（不合并）：Unity 专属，draw call 数不变只降 CPU → 免费附加，非主方案。
- GPU Instancing：Unity/Godot API 异构、与 SRP 互斥、Text 软肋 → 留 v2。
- egui 段式（无 AABB）：合批率取决于 sort_key 连续性，不如 AABB 重排。
- uGUI/UI Toolkit：Unity 专属，uGUI 无 AABB 重排（比 fgui 原始）。
- Slint：靠塞 Skia/FemtoVG 重型库，不适合「后端是游戏引擎」。

LoomGUI 因 **blob SOA + quad-only + 已图集**，做 fgui 核心比 fgui 原版更高效（合并是连续内存 cat vs `List<Vertex>` 堆分配；AABB 是 4 顶点 min/max 常数时间 vs 任意 mesh 遍历顶点）。**借鉴 fgui 算法核心 + 自己补显式合并，不照搬 fgui 实现**（深度耦合 Unity Material/Shader/MeshRenderer/sortingOrder/stencil）。

## 4. 架构总览（数据流）

```
build_render_nodes（不变）
   → FrameData{ nodes: Vec<RenderNode>, clips }
        ↓
assign_sort_keys（batch.rs 扩展）
   = mask_context 分层（已有）+ AABB 保序重排（新增）→ 重排 sort_key
        ↓
merge_meshes（render/merge.rs，新增）
   → 按 sort_key 扫描：连续同 DrawState 的 Mesh 节点 → 拼成单 merged Mesh payload
   → FrameData{ nodes: 变少, clips: 不变 }
        ↓
build_blob（blob.rs 不变）→ Unity MirrorPool（零改）
```

三处改动：**① `batch.rs` 加 AABB 重排 ② 新增 `render/merge.rs` ③ blob/MirrorPool 零改**。

## 5. DrawState（batch key）

`DrawState = (texture, program, mask_context)`。
- `texture`：tex_id（atlas=1，纯色 Container=0）。
- `program`：0=Mesh（Container/Button/Image），1=Text。
- `mask_context`：clip 层级（`batch.rs` 已算）。
- blend 当前仅 Normal，**不入 key**（以后加 blend 再入）。

两节点并入同 batch ⟺ DrawState 全同 ⟹ 才能 merge。

## 6. AABB 保序重排（fgui 核心，core 化）

照搬 fgui `Container.cs:877-941` 稳定插入排序，适配 LoomGUI：
- **重排元素 = program=0 的 Mesh RenderNode**。**Text（program=1）作为天然 batch break，保持 DFS 序不参与重排**——它不 merge（§10），重排无收益且有改绘制顺序风险。Text 在 sort_key 序列里作为断点，其前后两段 Mesh 各自独立重排。
- **BatchingRoot 边界**：`mask_context` 开新层的节点（clip Container）= BatchingRoot。重排在每个 root 子树内独立做，**不下钻子 root**（子 root 独立成批）。LoomGUI 的 mask_context 分层已是这个边界。
- **元素 AABB** = 该节点 `layout_rect`（绝对 design 坐标，quad AABB = rect 本身，**常数时间**）。
- **稳定插入排序**：每个 i 向前扫 j，找「同 DrawState 且 AABB 不相交」的位置插入（不相交才能安全前移，避免遮挡错乱）；相交则保持原序。
- **产出**：重排后顺序 → 重新分配全局递增 `sort_key`，供 merge 扫描。
- **复杂度**：最坏 O(n²)，但 quad AABB 廉价 + BatchingRoot 把 n 限到单 root 子树 + AABB 相交早退，实践 O(n·k)。**用 fgui 原版，不预优化**（sweep-and-prune 等留 §12）。

## 7. 显式 mesh 合并（`render/merge.rs`）

按 sort_key 顺序扫描，**连续同 DrawState 的 program=0 Mesh 节点** → 拼成单个 merged `Mesh` payload：

| 字段 | merged 产物 |
|---|---|
| `verts` | 各节点 verts 直接 cat（`build_render_nodes` 产的已是绝对 design 坐标，见 §9） |
| `uvs` | 各节点 uvs 直接 cat |
| `colors` | 各节点 colors cat，**每个的 alpha 分量 ×= 该节点 `rn.alpha`**（把 `blob.rs:90` 的烤制提前到 merge）；**rgb 不动**（color_tint 不传、不乘——前景/文本色，Container/Image 顶点色只用 background） |
| `indices` | 各节点 indices cat + 累加顶点偏移（第 k 个节点 += 前 k-1 节点顶点数） |
| `transform` | `(0, 0)`（verts 已绝对） |
| `texture/program` | batch 的（同 DrawState） |
| `mask_context` | batch 的（同层） |
| `alpha` | `1.0`（已烤进 colors，防 blob.rs 二次烤） |
| `node_id` | **锚节点 = batch 内最小原始 node_id**（§8 硬不变量） |
| `sort_key` | batch 在重排序列中的位置 |

Text（program=1）/ 不同 DrawState / 相交未并入 → 断段，保持独立 payload。

## 8. 锚 node_id（硬不变量）

**merged 节点 `node_id` = batch 内最小原始 node_id。** 解决动画场景的 GO 抖动：
- batch 划分不变时锚稳定 → MirrorPool `_pool[node_id]→GO` 复用 → **零 GO 抖动**。
- 元素移动但 batch 不变（最常见动画）：同 GO 复用，每帧 `SetVertices` 更新拼接后的 mesh。
- batch 划变（元素跨 batch，低频）：用「最小 node_id」作锚，多数情况仍稳定；极端才增删 GO。
- **MirrorPool 零改**：它只按 node_id 复用 GO，看不出 merged vs 单节点。

## 9. blob / Unity 零改论证（低风险核心）

- **`blob.rs` 零改**：merged transform=0 → re-base（减 transform.x/y，`blob.rs:70`）减 0 → verts 保持绝对 design。merged 仍是 `Mesh` payload（`payload_kind=1`，`mesh_off/len` 指向拼接后的大 segment）。**14 列 SOA 结构一字不改**，只是 `node_count` 少、单 segment 大。merged alpha=1 → blob.rs:90 烤 alpha×1=不变。
- **`FrameBlob.cs` / `MaterialManager.cs` 零改**：只看 payload_kind + mesh segment + `(texture, program, mask_context)` key，与单节点同形。
- **`MirrorPool.cs` 零改**：每节点仍一个 GO。merged transform=0 → GO 在 root 原点、verts 绝对 → 渲染正确；锚 node_id 稳定 → GO 复用；节点变少 → draw call 自然降。
- **SRP Batcher 免费附加**：shader 保 SRP-Batcher compatible，合并后剩余少数 draw call 间 CPU setup 进一步降。与合并不冲突（SRP Batcher 不要求合并，只要求同 shader）。

> 关键事实支撑零改：`build_render_nodes` 产出的 verts 已是绝对 design 坐标（`mesh.rs` 用 `layout_rect.x/y` 作 quad 左上角，layout solve 后 layout_rect 即绝对）。当前 `blob.rs:70` 的 re-base 才把它们变 node-local。所以 merge = 直接拼接绝对 verts，merged transform 设 0，re-base 减 0 成立。

## 10. 边界（明确不做）

- **Text 不 merge**（program=1，与 Mesh 不同 DrawState；且 Text mesh 在后端生成，core 合不了）。
- **跨 clip 不 merge**（mask_context 不同 → 不同 DrawState）。
- **不同 texture 不 merge**（tex_id 不同）。
- **AABB 相交的同 DrawState 不并入同 batch**（保序正确性，fgui 核心）——它们各自独立 draw，sort_key 经重排。
- **不碰 dirty/diff**：v1b.4 假设全量重传，每帧重排重合并。dirty 优化是独立 defer（knowledge §7 已记）。

## 11. 测试 + 验收

**core 纯算法测**（Rust，CI，确定性）：
- AABB 重排：同 DrawState 不相交→聚拢；相交→保序防遮挡；跨 BatchingRoot→独立。
- merge：连续同 DrawState Mesh→1 merged（顶点数/索引偏移/colors alpha 烤制对）；Text/不同 DrawState→断段独立。
- 锚 node_id：= batch 内最小原始 node_id；batch 划变时稳定性。
- 边界：clip 隔断、texture 不同、相交保序、alpha 烤制（rgb 不动）。

**Unity EditMode 测**：merged blob fixture → MirrorPool 产 fewer GO + 大 mesh（顶点数 = 4×原始节点数）。

**PlayMode 验收**（押用户）：FrameDebugger 看 draw call 数 —— 连续同 atlas sprite 段 → 1 draw call（对比 v1b.3 的 N）；GO 数 = batch 数（<< 节点数）。

## 12. defer / YAGNI（明确不做，实测不够再加）

- 动画元素 opt-out merge（标记高频元素不合并）。
- 增量 dirty merge/diff（同 knowledge §7「静态帧 dirty 优化」）。
- 段表协议（不需要——payload_kind 已区分 Mesh/Text，merge 只动 Mesh 节点）。
- AABB 高级优化（sweep-and-prune / 分桶）。用 fgui 原版 O(n²) 稳定插入排序。
- blend 进 DrawState key（当前仅 Normal）。
- 同字体 Text 合并（需后端改）。
- GPU instancing（v2 自建 renderer 时）。
