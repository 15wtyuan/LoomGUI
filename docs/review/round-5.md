# 第五轮对抗性审查归档 — 过度设计与复杂化

> **本轮反向审查**：前三轮问"缺什么/会怎么崩"，第四轮"硬化契约"，本轮问"哪些过度设计、该砍该 defer"。
> 三视角并行：**vs fgui 过度设计** / **投机前向兼容字段（YAGNI）** / **文档可读性**。三视角高度收敛——强信号。
> 处理结论：主文档重定位为"当前实现真相源（设计意图 + v1 契约）"，v1.x 机制全部搬 `roadmap/v1x-deferred.md`。主文档 787→~600 行。

---

## 统领决策：主文档重新定位（F1）

原定位"最终总设计（单一真相源），不含版本范围"导致主文档详写 v1.x/v2 的实现机制（slot 算法、两遍DFS、版本门控、预留字段…），这些细节会随实现变、现在没人验证、v1 读者用不上 → 纯负担。

**新定位**：按"定没定"切，不按"v 几"切——
- 主文档 = **当前将实现的真相**：设计意图 + v1 精确契约。
- v1.x/v2 = `roadmap/v1x-deferred.md`：能力意图 + 精确契约草稿。等实现验证后"毕业"回主文档。
- 主文档不无限膨胀（只收已定）；v1.x 契约有家（roadmap）；有清晰毕业机制。

---

## 砍项（v1.x 机制污染 v1 文档 → 搬 roadmap）

| # | 砍项 | 理由 | fgui 对照 |
|---|---|---|---|
| K1 | §8.9 契约版本化全套（contract_version/feature_flags/扩展列偏移/SemVer） | 无 v2 契约，纯投机前向兼容 | fgui 多引擎 10 年无渲染数据版本化 |
| K2 | slot_id + reuse_key + §13.2 虚拟化详模 + §14.6 ReuseKey | v1.x，v1 里 slot_id 全 None、reuse_key 退化 node_id | fgui GList 用裸索引+资源URL池，无 reuse_key 层 |
| K3 | §8.8 两遍DFS + Erase sort_key + Mask Write/Content/Erase + MaskMode | v1 只 rect mask，shape mask 机制 v1 死代码 | fgui 单遍DFS + stencil ref 翻倍，无 Erase 节点 |
| K4 | Node `gears:[Option<Gear>;10]` + `gear_locked` + §11.3/§11.4 详 | v1.x，每节点背 80 字节空槽 | （fgui 有，v1 不做→搬） |
| K5 | NativeHost 全套（payload variant/`<l-native>`/Node类型/push时机/§15 drain步） | v1.x | — |
| K6 | PaintTarget variant + paintingMode + §8.6 mask 4 模式（只留 rect） | v1 只 rect mask | — |
| K7 | TextLayout 投机字段：cluster 砍、font_id 降 per-run、advance 出 FFI | 改 Rust struct 一行的事，FFI 表没冻结没发 | — |
| K8 | 包格式：migrator chain/nextPos forward-compat/branches/highResolution | 没"旧包"可迁；v1 单分辨率无多语言 | fgui 内联 `buffer.version>=N`，无迁移器链 |

**保留**（三人共识值得留）：bearing_x/y（v1 摆 quad 要）、formatVersion 裸头、BlendMode 概念（不列 12）、DrawState/DrawFlags/mask_context（fgui 同款）、rect mask、§5.5 动态规则表（CSS DSL 必要代价，**非**过度——reviewer 特意澄清）、Unchanged variant（FFI 必要）、§7.4 MatchWidthOrHeight（已比 fgui 少）。

---

## 文档级清理（不碰设计）

- slot→node 不变量原说 4 遍 → v1 删 slot 自然消失，机制搬 roadmap 单处。
- §8.7 七连弹 → 重写成结构化约定。
- §8.8/§9.2 算法公式 → 主文档留不变量/原则，公式搬 roadmap（两遍DFS）/删（顶点装配留"glyph 绝对坐标"一句）。
- §9.1 三个 ⚠️/v1.x 框 → 搬 roadmap，主文档留契约 + 一句指针。
- §4.4 CSS 列表 → 主文档作设计规范、注明 v1 冻结子集见 roadmap §2（不重复）。
- §14.3 混 5 件事 → 拆"数据契约/内存所有权/读取约定"，ArrayPool 预算搬 v1。
- §14.4/§14.6 重复 §8 → 去重，Unity 章节回归"职责清单 + 指针"。
- §15 管线每步 cross-ref → 删，留 bare 步骤。
- §0 TL;DR → 加 AI 驱动核心动机（原 TL;DR 没体现 §1.1 首要准则）。
- "曾考虑X但…" ADR 句子 → 删 review-talk（决策理由在 decisions/）。
- §6.2/§4.2 → 标 v1 锚点（Container/Button/Image/Text），余标 (v1.x)。

---

## 本轮关键产出

1. **主文档重新定位**（F1）：从"永恒最终设计"改"当前实现真相源"。这是统领决策，使 K1-K8 成为统一机械操作。
2. **v1 读者只看 v1 机制**：主文档不再有 slot/reuse_key/两遍DFS/版本门控/预留字段/ NativeHost/paintingMode 等未实现机制的细节。
3. **v1.x 机制有家且带正确版本**：`v1x-deferred.md` 收留所有搬出的机制草稿，其中 gear_locked 带第四轮纠正后的**正确**机制（同步同栈帧，非跨帧）——并显式标注"第四轮曾误写为跨帧，那是错的"。
4. **诚实标注投机**：契约版本化、迁移器链、预留字段不再伪装成"必要契约"，明确"等真需要再加"。

## Ponytail 取舍记录

- 三视角高度收敛 → 信号强，大胆砍。
- K1-K8 全 defer（非消灭）：复杂度从主文档挪 roadmap，主文档干净、v1 读者轻载、投机字段不污染 schema。每项实现时可一行 struct edit 加回。
- 第四轮刚硬化的 T2/T12/T13 中，T2(slot)、T12(两遍DFS)、T13(feature_flags) 本轮判过度 → 搬 roadmap。**不矛盾**：第四轮"既然保留就得写对"，第五轮"这套机制不该在 v1 文档"——视角更高一层。T7(text dirty)、T1(gear_locked 纠正) 是 v1 正确性，保留。
