# 第二轮对抗性审查（v2.2 → v2.3 修订）

> 3 视角：修复验证 / 契约与 FFI 一致性深挖 / v1 可实施性与遗漏。
> 重点验证第一轮修复是否真正到位。结论已沉淀进主文档和 ADR。

## 🔴 找出并已修复

### 1. TextLayout 跨 FFI 是最大断点 → ADR-006
第一轮解了"UV 归后端"，但冒出新问题：核心要把三层嵌套 TextLayout 跨 FFI。修复：SOA 三表（glyphs/runs/lines）。

### 2. FFI tagged arena 落地细节全空 → ADR-007
公共头漏 payload(offset,len)（自相矛盾）；变长混合 arena 无布局；C# 解析空白。修复：按类型多 arena + 公共头补三元组 + Span<byte> 读 + byte 布局定死。

### 3. "沿用上帧 mesh"在 RenderNode 没落点 → ADR-007
payload 无 Unchanged 变体。修复：加 Unchanged 变体，后端见之不动渲染对象。

### 4. 二进制包打包器缺失（v1 开不了工的致命缺口）→ ADR-008
v1 要加载二进制包但打包器排 v2。修复：打包器提前到 v1。

### 5. 围栏 v1 子集没冻结 → v1-scope.md §2
tech lead 给了冻结模板。修复：v1-scope.md 冻结清单。

## 🟡 澄清项（已沉淀）
- stencil ref 职责重叠 → ADR-003（Mask 意图化，ref 出契约）
- slot 复用 + dirty 恒真 + tween/事件/Gear 归属 → §13.2 + ADR-014
- 文本 quad 四套度量一致性 → ADR-004（字体资产契约，同一 ttf）
- stencil 扁平 Vec 两遍 DFS → §8.7（Erase 排子树末尾 + 批合不跨 Erase）

## 🟢 范围建议（已采纳）
- 文本 v1 砍 BiDi/复杂 shaping → ADR-017
- v1 平台收窄 Win/Mac+Mono → ADR-017
- v1 工作量 7-10 人月，最被低估是 Unity 后端镜像同步层（~2000 行 C#）
- tech lead 列 15 项 v1 必做 Unity 胶水 → v1-scope.md §3

## 总评
v2.2 实打实进步。但"渲染树→FFI→镜像"链路按当前设计做不到 v1.x：TextLayout 跨 FFI（最大断点）、stencil 协议在扁平 Vec 语义断裂、arena 方案只有口号。v1（quad+文本+矩形 clip）能跑通。
