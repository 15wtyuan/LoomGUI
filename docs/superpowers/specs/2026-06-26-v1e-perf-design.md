# v1e FFI 同步热路径性能优化 设计

> **子轮**：v1e（v1 性能债务兑现轮，Codex 评审 §4.3 + §6.5）。
> **前置**：v1d 全收尾（v1d.5 ScrollPane 已代码完成 + 本机 356 测绿，待家里机 PlayMode 验收）。v1e 与家里机 v1d.5 验收**并行**——v1e 是 perf 优化、不动功能逻辑，回归面小。
> **依据**：v1-scope §4（冷帧/换页帧 FFI≤2ms）、Codex 评审 §4.3（FrameBlob 性能债务偿还）、§6.5（v1e 兑现性能债务）；主文档 §614-623（SOA blob 契约，`Unchanged` 一等公民）；v1a T7 基线（500 节点 ~5-8ms/帧朴素全量重传）；坑 10（.dll stale 行为验证）。

## 0. 决策（brainstorm 已定）

| # | 决策 | 选定 |
|---|---|---|
| 1 | 触发时机 | 现在就做（不等家里 v1d.5 验收，并行） |
| 2 | 范围 | 只做 FFI 同步热路径 4 项；其余债务推"撞墙再上" |
| 3 | dirty 粒度 | 逐节点 hash 比较 |
| 4 | 验收方式 | Rust criterion bench + 家里机 PlayMode Profiler 双轨 |

## 1. 范围（4 项债务 → 3 技术动作 + 1 验收动作）

| # | 动作 | 落点 | 契约改动 |
|---|---|---|---|
| 1 | 逐节点 dirty hash + `Unchanged` emit | `stage.rs` / `render/mod.rs` | 零 |
| 2 | 静态帧≈0 + 冷帧全 dirty emit ≤2ms | 同上（dirty 自然结果） | 零 |
| 3 | C# ArrayPool 帧拷贝零 GC | `LoomStage.cs`（+ `MirrorPool.cs` 视 alloc 热点） | 零 |
| 4 | Rust criterion bench + 家里机 Profiler 双轨验收 | `loomgui_core/benches/` + 家里机 | 零 |

**关键实证**：`NodePayload::Unchanged` 从 v0 预留——
- 设计文档 §619：`Unchanged 节点 payload_kinds=Unchanged，三元组为空`。
- `FrameBlob.cs:36`：blob 列 12 = `payload_kind(u8, 0=Unchanged 1=Mesh 2=Text)`。
- `MirrorPool.cs:71`：`if (kind != 1 && kind != 2) continue;`——遇 Unchanged 直接跳过 upload。

→ Rust stage 一旦产出 Unchanged，C# 自动跳过。**纯 Rust 侧优化，FFI/blob/C# 三零改动**。

## 2. 不做（明确推后，YAGNI）

撞墙再上的投机项，本轮不动：
- 文本测量 cache `(text,font,size,constraint)→(w,h)`（v1b §7，500 节点 naive 够）
- Font `Box::leak` 缓存化（v1b CJK，×20 域重载未现显著增长）
- invalidation set 伪类重匹配（v1c.1/v1c.4，全量重 cascade 当前可接受）
- Tab 链缓存（v1d.2，UI 规模可接受）
- AncestorChain 池化（v1c.4，Move 热路径）
- FairyBatching 实机优化验证（v1a §7，静态 500 已过便宜帧）

> AncestorChain 池化**不**进 v1e 的另一层理由：dirty hash 已天然捕获 transform 传播（祖先 transform 变 → 后代 world matrix 变 → hash 不等 → 重传），不需专门祖先链传播逻辑。

## 3. 动作 1：逐节点 dirty hash + Unchanged emit

### 3.1 dirty hash 字段集

每节点算一个 `u64` hash（FxHash / 默认 Hasher，帧内一致即可，不求抗碰撞——碰撞最坏是多余重传，不破正确性）。字段：

- **transform**：world matrix 6 列 `m_a..m_ty`（v1d.5 blob v4 列）。**含 scroll_pos**——scroll 容器子树 world matrix 随 scroll_pos 变 → hash 不等 → 自动重传，不需特殊处理。
- **style 关键字段**：`visible / alpha / grayed / color_tint / blend / mask_context / sort_key`（blob 公共头列）。
- **payload 摘要**（不 hash 全量顶点，太贵；hash 摘要 + count）：
  - Mesh：`tex_id` + 顶点数 + `colors[0]`（首色，捕获纯色块变色）。
  - Text：`glyph_count` + `font_size` + `color` + `glyphs[0].codepoint`（首字，捕获文本内容变）。
  - Unchanged（上帧就是 Unchanged）：hash = 0（强制重判，因无 payload 可比——但首帧后稳定节点会重新算出 Mesh/Text hash，下帧起正常比较）。

> **为什么 hash 摘要而非全量**：全量顶点 hash = 每帧 O(顶点数) 额外 CPU，500 节点 × 4 顶点 = 2000 f32 hash，可能吃掉 dirty 跳过省下的 upload 时间。摘要 O(1) per node，跳过 upload 的收益（Marshal.Copy + SetVertices）远大于摘要成本。

### 3.2 Stage 持有上帧 hash 表

```rust
// Stage 加 transient 字段（不进 pkg，同 scroll/anim）：
//   prev_node_hashes: Vec<u64>   // NodeId 索引，跨 tick 持续
// 首帧空 → 全 dirty（无基线，强制全 emit，正确性优先）。
// 节点数变（reload）→ 清空 → 下帧全 dirty。
```

- `load_inline` / `load_package` / `clear` 调 `prev_node_hashes.clear()`（同 scroll clear，防悬空/错位）。
- 坑 43 同源：给 Scene/Stage 加 transient 字段，相邻字段一次命中。

### 3.3 build_render_nodes 改造

现状（`render/mod.rs:76-92`）：预分配 Unchanged 占位，逐节点**无条件**覆写成 Mesh/Text。

改为：
1. 算当前帧每节点 hash（`compute_node_hash(scene, node_id, world_transforms, &prev_frame_payload)`）。
2. 与 `prev_node_hashes[node_id]` 比：
   - **不等**（或首帧/无基线）→ emit Mesh/Text（现状逻辑），记录新 hash。
   - **相等** → **保留 `NodePayload::Unchanged`**（占位即 Unchanged，不覆写），hash 不变。
3. 写 `prev_node_hashes = 当前帧 hash 表`（供下帧比）。

- **合成 scrollbar 节点**（v1d.5 §7，sentinel id）：scrollbar 随 scroll_pos 变（thumb 位置/尺寸每帧算），**强制每帧 emit**（不进 hash 比较，始终 Mesh）——scrollbar 本就只在滚动时显，且数量少（每 effective 容器 1-2 个），全 emit 无压力。
- **batch/merge 路径零改**：`Unchanged` 在 `batch.rs` / `merge.rs` 已正确处理（作 batch break、原样 passthrough，v0 已就位）。

### 3.4 正确性不变量

1. **首帧全 emit**（无基线）→ 与 v1d.5 末态逐节点等价（零回归基线）。
2. **任一输入/动画/scroll/伪类变化** → 受影响节点 hash 不等 → 重传 → 视觉正确。
3. **hash 碰撞**（两帧不同但 hash 同）→ 最坏该节点本帧不重传 → 1 帧视觉延迟。可接受（碰撞概率极低，且下一帧该节点若再变会重传）。**这是 ponytail 简化的已知天花板**：用 `// ponytail: hash 碰撞最坏 1 帧延迟，不破正确性；换全量 hash 若 profiling 显示遗漏` 标注。
4. **reload/节点数变** → 清 hash 表 → 下帧全 dirty → 正确。
5. **静态帧**（无任何变化）→ 全部 Unchanged → C# MirrorPool 全跳过 → 静态帧≈0 upload。

## 4. 动作 2：静态帧≈0 + 冷帧 ≤2ms（动作 1 的自然结果）

- **静态帧**：动作 1 后，未变节点全 Unchanged → C# `MirrorPool.cs:71` 全 `continue` → 零 `Marshal.Copy` / 零 `SetVertices` / 零 GO 操作。Rust 侧只剩 hash 计算 O(n)（500 节点 ~μs 级）。
- **冷帧/换页帧**（500 节点全 dirty）：全部 emit Mesh/Text。Rust 侧 build_render_nodes + emit blob 的耗时由 criterion bench 量化（§6），须 ≤2ms。C# 侧 upload 耗时由家里机 Profiler 读。

> "冷帧"= 首帧或 reload 后首帧（全无基线）。"换页帧"= Controller 全量换页（v1.x 才有 Controller；v1e 用"全节点 style 突变"模拟，如 demo 里一次性改 500 节点 background-color）。

## 5. 动作 3：C# ArrayPool 帧拷贝零 GC

### 5.1 LoomStage._frameBuf（必做，最大块）

现状（`LoomStage.cs:545`）：`if (_frameBuf == null || _frameBuf.Length < len) _frameBuf = new byte[len];`——每帧若 len 变就 new，且从不归还。

改：
```csharp
private byte[] _frameBuf;          // ArrayPool 租用
// Tick 中借帧：
if (_frameBuf == null || _frameBuf.Length < len) {
    if (_frameBuf != null) ArrayPool<byte>.Shared.Return(_frameBuf);
    _frameBuf = ArrayPool<byte>.Shared.Rent(len);
}
// borrow_frame → Marshal.Copy(ptr, _frameBuf, 0, len) → 解析
// LoomStage.OnDestroy / Domain reload reset：Return(_frameBuf)
```

- Rent 的数组可能比 len 长（取整到 2 的幂）——只 copy/解析 `len` 字节，多余忽略。
- **Domain reload 保护**（坑 13/G13）：`[RuntimeInitializeOnLoadMethod(SubsystemRegistration)]` reset 静态时，若 _frameBuf 是实例字段随 Stage 销毁；若提到静态则 reset 时 Return。

### 5.2 MirrorPool.ReadMesh（视 alloc 热点决定）

现状（`MirrorPool.cs` + `FrameBlob.cs:107-128`）：`ReadMesh` 返回 `MeshSegment` 持 `verts/uvs/colors/indices` 数组拷贝，每节点 new 4 个数组。

- **本轮判断**：静态帧动作 1 后 Unchanged 节点根本不调 ReadMesh → 静态帧零 alloc。冷帧全 dirty 才每节点 4 数组 alloc。
- **决策**：本轮**先只动 `_frameBuf`**（最大单一块，整帧拷贝）。ReadMesh 的 per-node 数组**留观察**——家里机 Profiler 若显示冷帧 GC 卡顿在 ReadMesh，再动（用 `List<Vector3>` 复用 + `SetVertices(List)` overload，v1a Phase2 注释已指出此路径）。
- 标 `// ponytail: ReadMesh per-node alloc 留观察，冷帧 GC 卡顿再上 List 复用`。

> **理由**：YAGNI。_frameBuf 是确定的每帧大块 alloc（len = 整个 blob），先砍它。ReadMesh 在静态帧已被 dirty 跳过归零，冷帧才暴露——是否卡顿要实机数据，不投机实现。

## 6. 动作 4：验收（Rust bench + 家里机 Profiler 双轨）

### 6.1 Rust criterion bench（本机交付）

`loomgui_core/benches/frame_emit.rs`：
- **静态帧**：500 节点稳定 UI，连续 tick 2 次，测第 2 次 `build_render_nodes`（全 Unchanged emit）耗时。预期 ~μs 级。
- **冷帧**：500 节点首帧 tick（全 dirty emit）。预期 ≤2ms。
- **换页帧**：500 节点，第 2 帧一次性全节点改 style（模拟换页），测该帧 emit 耗时。预期 ≤2ms。
- bench 不依赖 Unity（纯 Rust core），本机可跑。

### 6.2 家里机 PlayMode（家里机交付）

- Unity Profiler 读帧时间：静态帧 / 冷帧 / 换页帧的 `LoomStage.Tick` + `MirrorPool.Upload` 耗时。
- **验收线**：冷帧/换页帧 FFI 拷贝 + arena 解析 ≤2ms（v1-scope §4）；静态帧无卡顿（≈0 upload）。
- GC 验证：Profiler GC Alloc 静态帧≈0（_frameBuf ArrayPool 后无每帧 alloc）。

### 6.3 测覆盖

- **core 单测**：dirty hash 正确性（变字段→hash 不等→emit；不变→Unchanged）；首帧全 dirty；reload 清 hash；scroll_pos 变→子树重传；合成 scrollbar 强制 emit；hash 碰撞不崩（1 帧延迟）。
- **ffi abi 测**：version `v1e`；blob 仍 v4（零 bump 验证）；`Unchanged` kind 经 blob round-trip（C# 侧已有 `kind!=1&&!=2 continue`，本机 round-trip 测 kind=0 透传）。
- **回归**：既有 356 测全绿（build_render_nodes 改造的 fallout）。

## 7. FFI / version / 跨语言

### 7.1 version

- version 串 **v1d.5 → v1e**。
- **blob 保持 v4**（`Unchanged` kind 早有，stage 产出不改 blob 布局；MirrorPool/shader 零改）——**零 bump**。
- **pkg formatVersion 保持 7**（ResolvedStyle 无字段变）。
- .dll 重编 + commit（两机约束；坑 10）。

### 7.2 C# 侧

- `LoomStage._frameBuf` → ArrayPool（§5.1）。
- `MirrorPool` 零改（`kind!=1&&!=2 continue` 已就位，自动吃 Unchanged）。
- 无新 FFI 函数、无新 struct、无 EventType 新增。

## 8. 不变量 / 风险

**不变量**：
1. 首帧全 emit → 与 v1d.5 末态逐节点等价（零回归基线）。
2. 静态帧全部 Unchanged → C# 零 upload → 与 v1d.5 静态视觉等价（零回归）。
3. 任一变化 → 受影响节点重传 → 视觉正确（hash 天然捕获 transform/style/payload 摘要）。
4. blob v4 / pkg v7 / FFI 契约零改 / C# MirrorPool 零改。
5. dirty 跟踪 transient 不进 pkg（同 scroll/anim）。

**风险**：
1. **hash 碰撞 1 帧延迟**（ponytail 天花板）：u64 碰撞概率极低，不破正确性；profiling 显示遗漏再换全量 hash。
2. **hash 字段遗漏**：若某影响视觉的字段没进 hash → 该字段变不重传 → 持续视觉错（非 1 帧延迟，是持续错）。**这是真风险**——spec §3.1 字段集须逐项对照 blob 公共头列 + payload 摘要，reviewer 严查遗漏。scroll_pos 已由 world matrix hash 覆盖（实证）。
3. **合成 scrollbar 强制 emit**：若误进 hash 比较且 hash 碰撞 → scrollbar 不更新 → thumb 位置错。强制 emit 规避（§3.3）。
4. **ArrayPool 租用长度 > len**：解析须严格用 `len`，多余字节忽略（§5.1）。
5. **Domain reload ArrayPool 泄漏**：_frameBuf 若不 Return → 域重载累积。reset 钩子须 Return（§5.1，坑 13 同源）。
