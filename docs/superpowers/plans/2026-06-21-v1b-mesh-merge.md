# v1b.4 Mesh 合并 + AABB 保序重排 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** core 端把同 batch 的多 quad 拼成单个 mesh → 真 N→1 draw call（补 fgui 靠 Unity Dynamic Batching 隐式做的显式合并）。

**Architecture:** `batch.rs` 加 AABB 保序重排（fgui `DoFairyBatching` 稳定插入排序）→ 新增 `render/merge.rs` 按重排后 sort_key 把连续同 DrawState 的 Mesh 节点拼成单 merged `Mesh` payload（verts 绝对、transform=0、colors 烤 alpha、index 偏移、锚 node_id）→ blob v3 / MirrorPool 零改（merged transform=0/alpha=1 让 re-base 与 alpha 烤制对 merged 失效）。

**Tech Stack:** Rust（loomgui_core + loomgui_ffi_c）、Unity 6.5 URP（C#）、csbindgen。

## Global Constraints

- **blob v3 列结构零改**（14 列 SOA，`blob.rs:20-26`）；`blob.rs:70` re-base（verts 减 transform）+ `blob.rs:90` alpha 烤制（`c[3]*rn.alpha`）逻辑不动 —— 靠 merged 节点 `transform=(0,0)`/`alpha=1.0` 使其对 merged 无效。
- **锚 node_id 硬不变量**：merged 节点 `node_id` = batch 内最小原始 node_id（解决动画 GO 抖动，MirrorPool `_pool[node_id]→GO` 复用稳定）。
- **AABB 重排**照搬 fgui `Container.cs:877-941` 稳定插入排序，**不预优化**（不做 sweep-and-prune / 分桶）。
- **Text（program=1）不重排不合并**（batch break）；只重排/合并 `program=0` 的 Mesh 节点。
- **DrawState = (texture, program, mask_context)**；blend 当前仅 Normal 不入 key。
- **AABB 来源** = `scene.nodes[i].layout_rect`（绝对 design `Rect{x,y,w,h}`，quad AABB=rect 本身）。
- **colors 烤制**：merge 时每原始节点 `colors[a] *= rn.alpha`（rgb 不动，color_tint 不传不乘），merged `alpha=1.0`。
- **YAGNI**：不做动画 opt-out、增量 dirty、段表协议、AABB 高级优化、blend 入 key、同字体 Text 合并。
- **测试在 core**（Rust CI，确定性）；Unity EditMode 测用真断言（非 `Assert.Pass`）；PlayMode 验收押用户（FrameDebugger draw call 数）。
- 模型策略：T1-T5 sonnet；task reviewer sonnet；final whole-branch review opus。
- 共享文件顺序编辑：`batch.rs`(T1) → `merge.rs`(T2) → `mod.rs`+`blob.rs`(T3) → Unity(T4) → sample+.dll(T5)。

---

### Task 1: AABB 保序重排（`render/batch.rs`）

**Files:**
- Modify: `loomgui_core/src/render/batch.rs`（加 `reorder_for_batching` + helpers，末尾追加；`assign_sort_keys`/`rect_intersect` 不动）
- Test: 同文件 `#[cfg(test)] mod tests` 追加

**Interfaces:**
- Consumes: `Scene`（`scene.nodes[i].layout_rect` 作 AABB）、`RenderNode`（`sort_key`/`mask_context`/`payload`，由 `assign_sort_keys` 已赋）、`rect_intersect`（batch.rs:23，复用作 AABB 相交判断）、`NodePayload`/`MaskContext`（render::node）
- Produces: `pub fn reorder_for_batching(scene: &Scene, nodes: &mut [RenderNode])` —— 原地重排 `nodes` 的 `sort_key`（重排后序），供 T2 merge 扫描。helpers `is_mergeable_mesh`/`draw_state`/`reorder_unit` 为私有。

**算法**（fgui `Container.cs:877-941` 忠实移植）：
1. 按 `sort_key`（DFS 序）排索引得 `order: Vec<usize>`。
2. 一遍扫描识别**重排单元** = 连续的 `is_mergeable_mesh` 且同 `mask_context`；遇 Text/`mask_context` 变 → 断。
3. 单元内 `reorder_unit`（fgui 稳定插入排序：i 向前扫 j，同 DrawState 且 AABB 不相交 → 前移到 k；相交保序防遮挡）。
4. 全局按 DFS 序重赋 `sort_key`（单元内按重排后顺序）。

- [ ] **Step 1: 加 helpers + `reorder_unit` 失败测先写**

在 `batch.rs` 的 `use` 区下方（`rect_intersect` 之后、`assign_sort_keys` 之前）追加：

```rust
/// 是否可合并 Mesh（program=0）。Text（program=1）/ Unchanged 不参与重排与合并。
fn is_mergeable_mesh(rn: &RenderNode) -> bool {
    matches!(&rn.payload, NodePayload::Mesh { program, .. } if *program == 0)
}

/// 可合并 Mesh 的 DrawState = (texture, mask_context)。
/// （program 已由 is_mergeable_mesh 保证 0；blend 仅 Normal 不入 key。）
/// 非 mergeable Mesh / Text / Unchanged → None。
fn draw_state(rn: &RenderNode) -> Option<(u32, u32)> {
    match &rn.payload {
        NodePayload::Mesh { texture, program, .. } if *program == 0 => {
            Some((*texture, rn.mask_context.0))
        }
        _ => None,
    }
}

/// AABB 是否重叠（交集非零面积）。复用 rect_intersect（batch.rs:23）。
fn aabb_overlap(a: Rect, b: Rect) -> bool {
    let r = rect_intersect(a, b);
    r.w > 0.0 && r.h > 0.0
}

/// 一个重排单元内做 fgui 式稳定插入排序（Container.cs:877-941）。
/// `unit` = 该单元内节点的 scene 索引（进入时为 DFS 序）；原地重排为 batch 聚拢后顺序。
fn reorder_unit(scene: &Scene, nodes: &[RenderNode], unit: &mut Vec<usize>) {
    let n = unit.len();
    if n < 2 {
        return;
    }
    for i in 1..n {
        let cur = unit[i];
        let cur_ds = match draw_state(&nodes[cur]) {
            Some(d) => d,
            None => continue, // 单元内应全是 mergeable；防御
        };
        let cur_aabb = scene.nodes[cur].layout_rect;
        let mut k: Option<usize> = None; // 插入点（unit 内下标）
        let mut last_ds: Option<(u32, u32)> = None;
        let mut m = i;
        for j in (0..i).rev() {
            let test = unit[j];
            let test_ds = draw_state(&nodes[test]).unwrap(); // 单元内必 mergeable
            if last_ds != Some(test_ds) {
                last_ds = Some(test_ds);
                m = j + 1;
            }
            if cur_ds == test_ds {
                k = Some(m);
            }
            if aabb_overlap(cur_aabb, scene.nodes[test].layout_rect) {
                if k.is_none() {
                    k = Some(m);
                }
                break; // 相交保序，停止前扫
            }
        }
        if let Some(ki) = k {
            if ki != i {
                let item = unit.remove(i);
                unit.insert(ki, item);
            }
        }
    }
}
```

在 `tests` mod 末尾追加（先写 `reorder_unit` 测，验证算法核心）：

```rust
    use crate::render::node::{NodePayload, NodeTransform, MaskContext};

    /// 构造 program=0 Mesh RenderNode（给 reorder_unit 直接喂 unit 索引对应的 nodes）。
    fn mesh_rn(tex: u32, rect: Rect, mask: u32) -> RenderNode {
        RenderNode {
            node_id: 0,
            parent_id: None,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            transform: NodeTransform::default(),
            blend: BlendMode::Normal,
            mask_context: MaskContext(mask),
            sort_key: 0,
            payload: NodePayload::Mesh {
                verts: vec![[rect.x, rect.y], [rect.x + rect.w, rect.y],
                            [rect.x + rect.w, rect.y + rect.h], [rect.x, rect.y + rect.h]],
                uvs: vec![[0.0, 0.0]; 4],
                colors: vec![[1.0; 4]; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                texture: tex,
                program: 0,
            },
        }
    }

    #[test]
    fn reorder_unit_same_drawstate_disjoint_gathers() {
        // [A(tex1, x=0), B(tex2, x=100), C(tex1, x=200)] 全不相交 → C 前移到 A 旁。
        // scene.nodes 与 nodes vec 同序同长（reorder_unit 用 scene.nodes[idx].layout_rect 查 AABB）。
        let mut scene = Scene { roots: vec![], nodes: vec![] };
        scene.nodes.push({ let mut n = Node::default(); n.layout_rect = Rect{x:0.0,y:0.0,w:10.0,h:10.0}; n });
        scene.nodes.push({ let mut n = Node::default(); n.layout_rect = Rect{x:100.0,y:0.0,w:10.0,h:10.0}; n });
        scene.nodes.push({ let mut n = Node::default(); n.layout_rect = Rect{x:200.0,y:0.0,w:10.0,h:10.0}; n });
        let nodes = vec![
            mesh_rn(1, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, 0),
            mesh_rn(2, Rect { x: 100.0, y: 0.0, w: 10.0, h: 10.0 }, 0),
            mesh_rn(1, Rect { x: 200.0, y: 0.0, w: 10.0, h: 10.0 }, 0),
        ];
        let mut unit = vec![0usize, 1, 2];
        reorder_unit(&scene, &nodes, &mut unit);
        // A,C 同 tex1 聚拢：[A(0), C(2), B(1)]
        assert_eq!(unit, vec![0, 2, 1], "同 DrawState 不相交 → C 前移到 A 旁");
    }

    #[test]
    fn reorder_unit_overlapping_keeps_order() {
        // A(tex1) B(tex2) C(tex1)，但 A 与 C AABB 相交 → C 不前移（保序防遮挡）。
        let mut scene = Scene { roots: vec![], nodes: vec![] };
        scene.nodes.push({ let mut n = Node::default(); n.layout_rect = Rect{x:0.0,y:0.0,w:50.0,h:50.0}; n });
        scene.nodes.push({ let mut n = Node::default(); n.layout_rect = Rect{x:100.0,y:0.0,w:10.0,h:10.0}; n });
        scene.nodes.push({ let mut n = Node::default(); n.layout_rect = Rect{x:10.0,y:10.0,w:50.0,h:50.0}; n });
        let nodes = vec![
            mesh_rn(1, Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 }, 0),
            mesh_rn(2, Rect { x: 100.0, y: 0.0, w: 10.0, h: 10.0 }, 0),
            mesh_rn(1, Rect { x: 10.0, y: 10.0, w: 50.0, h: 50.0 }, 0), // 与 A 相交
        ];
        let mut unit = vec![0usize, 1, 2];
        reorder_unit(&scene, &nodes, &mut unit);
        assert_eq!(unit, vec![0, 1, 2], "AABB 相交 → 保序不动");
    }
```

- [ ] **Step 2: 跑测验证失败**

Run: `cargo test -p loomgui_core render::batch::tests::reorder_unit`
Expected: 编译失败（`reorder_unit` 还没定义——Step 1 已写实现，但若先写测再写实现则 FAIL "cannot find function"；本 plan Step1 同步给实现+测，故应直接 PASS。若 reviewer 要求严格 RED，可先注释实现只留测跑一次确认编译错，再放开）。

实际：Step 1 实现与测同批写入，跑测应 PASS。若想严格 TDD，把 `reorder_unit` 函数体先换成 `let _ = (scene, nodes, unit);` 跑一次确认 FAIL，再恢复。

Run: `cargo test -p loomgui_core render::batch::tests::reorder_unit_same_drawstate_disjoint_gathers`
Expected: PASS

- [ ] **Step 3: 加 `reorder_for_batching` 公共入口 + 单元识别/重赋 sort_key 测**

在 `reorder_unit` 之后追加公共入口：

```rust
/// AABB 保序重排（spec §6）：按 BatchingRoot（mask_context）分段，段内对 program=0
/// Mesh 节点做 fgui 式稳定插入排序（同 DrawState + AABB 不相交才前移），重排后重赋
/// sort_key。Text（program=1）/ Unchanged 作为 batch break，不重排。
///
/// 前置：`assign_sort_keys` 已赋 mask_context + DFS 序 sort_key + clip 表。
/// 原地改写 `nodes[*].sort_key` 为重排后序。clips 表由 assign_sort_keys 产，不受影响。
pub fn reorder_for_batching(scene: &Scene, nodes: &mut [RenderNode]) {
    // 1. 按 sort_key（DFS 序）排索引。
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|&i| nodes[i].sort_key);

    // 2. 一遍扫描：识别重排单元（连续 mergeable + 同 mask_context）→ 重排 → 重赋 sort_key。
    let mut next_key: u32 = 0;
    let mut i = 0;
    while i < order.len() {
        let idx = order[i];
        if is_mergeable_mesh(&nodes[idx]) {
            let ctx = nodes[idx].mask_context;
            let mut unit: Vec<usize> = vec![idx];
            let mut j = i + 1;
            while j < order.len()
                && is_mergeable_mesh(&nodes[order[j]])
                && nodes[order[j]].mask_context == ctx
            {
                unit.push(order[j]);
                j += 1;
            }
            reorder_unit(scene, nodes, &mut unit);
            for &uidx in &unit {
                nodes[uidx].sort_key = next_key;
                next_key += 1;
            }
            i = j;
        } else {
            // Text / Unchanged：break，不重排，顺序赋 sort_key。
            nodes[idx].sort_key = next_key;
            next_key += 1;
            i += 1;
        }
    }
}
```

在 tests mod 追加 `reorder_for_batching` 测（跨单元 / Text break / 锚定顺序）：

```rust
    #[test]
    fn reorder_splits_at_text_break() {
        // root > [A(tex1), Text, B(tex1)]：AABB 全不相交。Text 断单元 →
        // A、B 分属两个单元，B 不能跨 Text 前移到 A 旁（保 Text 绘制序）。
        let mut scene = Scene { roots: vec![NodeId(0)], nodes: vec![] };
        let mut root = Node::default(); root.id = NodeId(0);
        root.children = vec![NodeId(1), NodeId(2), NodeId(3)];
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 300.0, h: 50.0 };
        scene.nodes.push(root);
        let mut a = Node::default(); a.id = NodeId(1);
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }; scene.nodes.push(a);
        let mut t = Node::default(); t.id = NodeId(2); t.kind = NodeKind::Text { content: "x".into() };
        t.layout_rect = Rect { x: 100.0, y: 0.0, w: 10.0, h: 10.0 }; scene.nodes.push(t);
        let mut b = Node::default(); b.id = NodeId(3);
        b.layout_rect = Rect { x: 200.0, y: 0.0, w: 10.0, h: 10.0 }; scene.nodes.push(b);

        let mut rns: Vec<RenderNode> = vec![
            { let mut r = placeholder_rn(0); r.payload = NodePayload::Unchanged; r.mask_context = MaskContext(0); r },
            mesh_rn_into_rn(1, 1, &scene), // tex1
            text_rn(2),
            mesh_rn_into_rn(3, 1, &scene), // tex1
        ];
        // 先赋 DFS 序 sort_key（模拟 assign_sort_keys 输出）+ mask_context。
        for (k, r) in rns.iter_mut().enumerate() { r.sort_key = k as u32; r.mask_context = MaskContext(0); }

        reorder_for_batching(&scene, &mut rns);
        // Text(id=2) 必在 A(id=1) 与 B(id=3) 之间（保绘制序）。
        let sk = |id: u32| rns.iter().find(|r| r.node_id == id).unwrap().sort_key;
        assert!(sk(1) < sk(2), "A 在 Text 前");
        assert!(sk(2) < sk(3), "Text 在 B 前（B 不跨 Text 前移）");
    }

    #[test]
    fn reorder_splits_at_mask_context_boundary() {
        // 两个 mask_context 的 Mesh 不跨边界重排（不同 DrawState）。
        // A(ctx0,tex1) B(ctx1,tex1) C(ctx0,tex1)：A、C 同 ctx0 但被 B(ctx1) 断开，
        // 且 AABB 不相交。C 不应跨 ctx 边界前移到 A 旁。
        let mut scene = Scene { roots: vec![NodeId(0)], nodes: vec![] };
        let mut root = Node::default(); root.id = NodeId(0);
        root.children = vec![NodeId(1), NodeId(2), NodeId(3)];
        scene.nodes.push(root);
        scene.nodes.push({ let mut n = Node::default(); n.id = NodeId(1); n.layout_rect = Rect{x:0.0,y:0.0,w:10.0,h:10.0}; n });
        scene.nodes.push({ let mut n = Node::default(); n.id = NodeId(2); n.layout_rect = Rect{x:100.0,y:0.0,w:10.0,h:10.0}; n });
        scene.nodes.push({ let mut n = Node::default(); n.id = NodeId(3); n.layout_rect = Rect{x:200.0,y:0.0,w:10.0,h:10.0}; n });

        let mut rns: Vec<RenderNode> = vec![
            mesh_rn_into_rn(1, 1, &scene),
            mesh_rn_into_rn(2, 1, &scene),
            mesh_rn_into_rn(3, 1, &scene),
        ];
        // sort_key = DFS 序；mask_context: 0→ctx0, 1→ctx1, 2→ctx0（模拟跨 clip 边界）。
        rns[0].sort_key = 0; rns[0].mask_context = MaskContext(0);
        rns[1].sort_key = 1; rns[1].mask_context = MaskContext(1);
        rns[2].sort_key = 2; rns[2].mask_context = MaskContext(0);

        reorder_for_batching(&scene, &mut rns);
        // C(ctx0) 不跨 B(ctx1) 前移：B 的 sort_key 仍在 A、C 之间或 A 前，但 C 不越 B。
        // 关键断言：A 与 C 不相邻聚拢越过 B——B(node_id=2) 的 sort_key < C(node_id=3) 前移后的位置不可能。
        let sk = |id: u32| rns.iter().find(|r| r.node_id == id).unwrap().sort_key;
        // C 不应跑到 B 前面（不同 ctx 不跨边界）。
        assert!(sk(2) < sk(3), "C(ctx0) 不跨 B(ctx1) 边界前移");
    }

    /// helper：把 mesh_rn 包成 RenderNode 并设 node_id。
    fn mesh_rn_into_rn(id: usize, tex: u32, _scene: &Scene) -> RenderNode {
        let mut r = mesh_rn(tex, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, 0);
        r.node_id = id as u32;
        r
    }
    fn text_rn(id: usize) -> RenderNode {
        let mut r = placeholder_rn(id);
        r.node_id = id as u32;
        r.payload = NodePayload::Text {
            layout: crate::text::layout::TextLayout { text_width: 0.0, text_height: 0.0, lines: vec![] },
            font_size: 16.0, color: [1.0; 4], program: 1,
        };
        r
    }
```

- [ ] **Step 4: 跑全部 batch 测**

Run: `cargo test -p loomgui_core render::batch`
Expected: 全 PASS（含原有 `keys_monotonic` 等，因 placeholder 是 Unchanged 不重排，sort_key 行为不变）

- [ ] **Step 5: workspace 绿 + commit**

Run: `cargo test --workspace`（注意：删 pub API 必须验全 workspace，knowledge 坑21；本 task 只新增函数不改签名，但仍验 workspace 防 ripple）
Expected: 全 PASS

```bash
git add loomgui_core/src/render/batch.rs
git commit -m "feat(v1b.4): AABB 保序重排（render::batch::reorder_for_batching）"
```

---

### Task 2: mesh 合并（`render/merge.rs` 新文件）

**Files:**
- Create: `loomgui_core/src/render/merge.rs`
- Modify: `loomgui_core/src/render/mod.rs`（`pub mod merge;` 注册，仅一行）
- Test: `merge.rs` 内 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `RenderNode`/`NodePayload`/`NodeTransform`/`MaskContext`（render::node）、T1 的同 DrawState语义（merge 不调 T1 函数，但依赖 T1 已把同 DrawState 不相交元素排到 sort_key 相邻）
- Produces: `pub fn merge_meshes(nodes: Vec<RenderNode>) -> Vec<RenderNode>` —— 按 sort_key 扫描，连续同 DrawState 的 program=0 Mesh → 拼成单 merged `Mesh` payload；Text/不同 DrawState/单节点单元保持独立。

**合并规则**（spec §7 + §8）：
- 输入 `nodes` 先按 `sort_key` 排序（T1 已重排）。
- 连续同 DrawState（`(texture, program, mask_context)`，program=0）的 Mesh → 1 merged。
- merged `verts`/`uvs`/`colors` = 各节点 cat；`colors[a] *= 各节点 alpha`；`indices` cat + 顶点偏移；`transform=(0,0)`；`alpha=1.0`；`texture/program/mask_context` = batch 的；`sort_key` = batch 首节点的；**`node_id` = batch 内最小原始 node_id（锚）**。
- Text（program=1）/ 不同 DrawState / Unchanged → 原样保留（独立节点）。

- [ ] **Step 1: 注册模块 + 写 merge_meshes 骨架与失败测**

`mod.rs` 在 `pub mod mesh;` 后加一行：

```rust
pub mod merge;
```

创建 `merge.rs`：

```rust
//! Mesh 合并（spec §7）：按 sort_key 扫描，连续同 DrawState 的 program=0 Mesh 节点
//! 拼成单个 merged Mesh payload → 1 draw call。
//!
//! 前置：`batch::reorder_for_batching` 已把同 DrawState 不相交元素排到 sort_key 相邻。
//! Text（program=1）/ 不同 DrawState / Unchanged 保持独立。

use crate::render::node::{MaskContext, NodePayload, NodeTransform, RenderNode};

/// DrawState 键（texture, program, mask_context）。program=0 Mesh 才参与合并。
fn mesh_key(rn: &RenderNode) -> Option<(u32, u32, u32)> {
    match &rn.payload {
        NodePayload::Mesh { texture, program, .. } if *program == 0 => {
            Some((*texture, *program, rn.mask_context.0))
        }
        _ => None,
    }
}

/// 按 sort_key 扫描，连续同 DrawState 的 Mesh 节点合并成单个 merged Mesh payload。
/// merged node_id = batch 内最小原始 node_id（锚，spec §8）。
pub fn merge_meshes(nodes: Vec<RenderNode>) -> Vec<RenderNode> {
    // 1. 按 sort_key 排序（T1 重排后序）。
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|&i| nodes[i].sort_key);

    let mut out: Vec<RenderNode> = Vec::with_capacity(nodes.len());
    let mut i = 0;
    while i < order.len() {
        let idx = order[i];
        let key = mesh_key(&nodes[idx]);
        if key.is_none() {
            // Text / Unchanged：原样。
            out.push(nodes[idx].clone());
            i += 1;
            continue;
        }
        let key = key.unwrap();
        // 收集连续同 key 的 Mesh。
        let mut batch_idx: Vec<usize> = vec![idx];
        let mut j = i + 1;
        while j < order.len() && mesh_key(&nodes[order[j]]) == Some(key) {
            batch_idx.push(order[j]);
            j += 1;
        }
        if batch_idx.len() == 1 {
            out.push(nodes[idx].clone());
        } else {
            out.push(merge_batch(&nodes, &batch_idx));
        }
        i = j;
    }
    out
}

/// 把一组同 DrawState Mesh 节点拼成单个 merged Mesh payload。
fn merge_batch(nodes: &[RenderNode], batch: &[usize]) -> RenderNode {
    // 锚 node_id = batch 内最小原始 node_id。
    let anchor = batch.iter().map(|&i| nodes[i].node_id).min().unwrap();
    let last = &nodes[*batch.last().unwrap()]; // 取 texture/program/mask_context/sort_key 模板
    let mut verts: Vec<[f32; 2]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut base: u32 = 0;
    for &bi in batch {
        if let NodePayload::Mesh { verts: v, uvs: u, colors: c, indices: ix, .. } = &nodes[bi].payload {
            let alpha = nodes[bi].alpha;
            verts.extend_from_slice(v);
            uvs.extend_from_slice(u);
            // colors：alpha 分量 ×= 节点 alpha（rgb 不动）；merged alpha=1 防 blob 二次烤。
            for cc in c {
                let mut col = *cc;
                col[3] *= alpha;
                colors.push(col);
            }
            for &ixv in ix {
                indices.push(ixv + base);
            }
            base += v.len() as u32;
        }
    }
    RenderNode {
        node_id: anchor,
        parent_id: None,
        visible: true,
        alpha: 1.0,
        grayed: false,
        color_tint: [1.0; 4],
        transform: NodeTransform { x: 0.0, y: 0.0, ..NodeTransform::default() },
        blend: last.blend,
        mask_context: last.mask_context,
        sort_key: last.sort_key,
        payload: NodePayload::Mesh {
            verts, uvs, colors, indices,
            texture: match &last.payload {
                NodePayload::Mesh { texture, .. } => *texture,
                _ => 0,
            },
            program: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::node::BlendMode;

    fn mesh_node(id: u32, tex: u32, sort_key: u32, alpha: f32, rect_off: f32) -> RenderNode {
        RenderNode {
            node_id: id, parent_id: None, visible: true, alpha,
            grayed: false, color_tint: [1.0; 4],
            transform: NodeTransform::default(),
            blend: BlendMode::Normal, mask_context: MaskContext(0), sort_key,
            payload: NodePayload::Mesh {
                verts: vec![[rect_off, 0.0], [rect_off + 10.0, 0.0],
                            [rect_off + 10.0, 10.0], [rect_off, 10.0]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                colors: vec![[1.0, 1.0, 1.0, 1.0]; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                texture: tex, program: 0,
            },
        }
    }
}
```

- [ ] **Step 2: 写合并正确性测（失败）**

在 `merge.rs` 的 `tests` mod 追加：

```rust
    #[test]
    fn two_same_drawstate_merge_into_one() {
        // A(tex1,sk0) B(tex1,sk1) → 1 merged：8 verts / 12 indices / colors alpha 烤制。
        let nodes = vec![
            mesh_node(5, 1, 0, 1.0, 0.0),
            mesh_node(3, 1, 1, 0.5, 100.0), // alpha=0.5
        ];
        let out = merge_meshes(nodes);
        assert_eq!(out.len(), 1, "2 同 DrawState → 1 merged");
        match &out[0].payload {
            NodePayload::Mesh { verts, indices, colors, texture, .. } => {
                assert_eq!(verts.len(), 8, "2×4 verts");
                assert_eq!(indices.len(), 12, "2×6 indices");
                assert_eq!(*texture, 1);
                // 第二个节点（alpha=0.5）的 4 顶点 colors[4..8].a == 0.5。
                for c in &colors[4..8] {
                    assert!((c[3] - 0.5).abs() < 1e-6, "第二节点 alpha=0.5 烤进 colors.a");
                }
                // 第一个节点（alpha=1）colors[0..4].a == 1.0。
                for c in &colors[0..4] {
                    assert!((c[3] - 1.0).abs() < 1e-6);
                }
            }
            _ => panic!("expected Mesh"),
        }
        // 锚 node_id = min(5,3) = 3。
        assert_eq!(out[0].node_id, 3, "锚 = batch 内最小 node_id");
        // merged transform=0 / alpha=1。
        assert_eq!(out[0].transform.x, 0.0);
        assert_eq!(out[0].transform.y, 0.0);
        assert!((out[0].alpha - 1.0).abs() < 1e-6);
    }

    #[test]
    fn index_offset_correct_for_three_nodes() {
        // 3 节点同 DrawState → merged indices 第二组 +4、第三组 +8。
        let nodes = vec![
            mesh_node(1, 1, 0, 1.0, 0.0),
            mesh_node(2, 1, 1, 1.0, 50.0),
            mesh_node(3, 1, 2, 1.0, 100.0),
        ];
        let out = merge_meshes(nodes);
        assert_eq!(out.len(), 1);
        if let NodePayload::Mesh { indices, .. } = &out[0].payload {
            // 第一组 [0,1,2,0,2,3]，第二组 +4 [4,5,6,4,6,7]，第三组 +8 [8,9,10,8,10,11]。
            assert_eq!(indices, &vec![0u32,1,2,0,2,3, 4,5,6,4,6,7, 8,9,10,8,10,11]);
        } else { panic!("expected Mesh"); }
    }

    #[test]
    fn different_drawstate_stay_separate() {
        // A(tex1) B(tex2) 同 mask_context 但 texture 不同 → 不合并。
        let nodes = vec![mesh_node(1, 1, 0, 1.0, 0.0), mesh_node(2, 2, 1, 1.0, 100.0)];
        let out = merge_meshes(nodes);
        assert_eq!(out.len(), 2, "不同 texture → 各自独立");
    }

    #[test]
    fn text_node_stays_separate() {
        let mut mesh = mesh_node(1, 1, 0, 1.0, 0.0);
        let mut text = mesh_node(2, 1, 1, 1.0, 100.0);
        text.payload = NodePayload::Text {
            layout: crate::text::layout::TextLayout { text_width: 0.0, text_height: 0.0, lines: vec![] },
            font_size: 16.0, color: [1.0; 4], program: 1,
        };
        let out = merge_meshes(vec![mesh, text]);
        assert_eq!(out.len(), 2, "Text 不参与合并");
    }
```

- [ ] **Step 3: 跑测验证通过**

Run: `cargo test -p loomgui_core render::merge`
Expected: 全 PASS

- [ ] **Step 4: workspace 绿 + commit**

Run: `cargo test --workspace`
Expected: 全 PASS

```bash
git add loomgui_core/src/render/merge.rs loomgui_core/src/render/mod.rs
git commit -m "feat(v1b.4): mesh 合并（render::merge::merge_meshes）"
```

---

### Task 3: 接入 `build_render_nodes` + blob round-trip 验证

**Files:**
- Modify: `loomgui_core/src/render/mod.rs:139-141`（assign_sort_keys 后加 reorder + merge）
- Modify: `loomgui_ffi_c/src/blob.rs` 测试区（加 merged FrameData round-trip 测，blob.rs 本体不动）
- Test: 两处新增测

**Interfaces:**
- Consumes: T1 `batch::reorder_for_batching`、T2 `merge::merge_meshes`
- Produces: `build_render_nodes` 现在返回的 `FrameData.nodes` 是 merged 后的（node_count 少）。blob/MirrorPool 契约不变。

- [ ] **Step 1: 接入 reorder + merge**

`mod.rs:139-141` 当前：
```rust
    let clips = batch::assign_sort_keys(scene, &mut nodes);
    FrameData { nodes, clips }
```
改为：
```rust
    let clips = batch::assign_sort_keys(scene, &mut nodes);
    batch::reorder_for_batching(scene, &mut nodes);
    let nodes = merge::merge_meshes(nodes);
    FrameData { nodes, clips }
```

- [ ] **Step 2: 端到端测——build_render_nodes 输出 merged**

在 `mod.rs` 的 `tests` mod 追加：

```rust
    #[test]
    fn build_merges_adjacent_same_drawstate_meshes() {
        // root > [img A(src=a), img B(src=a)]：同 tex_id=1、同 mask_context、AABB 不相交。
        // build_render_nodes → reorder 让其相邻 → merge 成 1 个 merged 节点。
        let mut scene = Scene { roots: vec![NodeId(0)], nodes: vec![] };
        let mut root = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 300.0, h: 50.0 }, None);
        root.children = vec![NodeId(1), NodeId(2)];
        scene.nodes.push(root);
        let mut a = Node::default(); a.id = NodeId(1); a.parent = Some(NodeId(0));
        a.kind = NodeKind::Image { src: "a.png".into() };
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        scene.nodes.push(a);
        let mut b = Node::default(); b.id = NodeId(2); b.parent = Some(NodeId(0));
        b.kind = NodeKind::Image { src: "a.png".into() };
        b.layout_rect = Rect { x: 100.0, y: 0.0, w: 10.0, h: 10.0 };
        scene.nodes.push(b);

        let font = test_font().expect("need test font");
        let mut tex = TextureRegistry::default();
        tex.insert("a.png", TexMeta { tex_id: 1, uv_min: [0.0,0.0], uv_max: [1.0,1.0], width: 10, height: 10 });

        let frame = build_render_nodes(&scene, &font, &tex);
        // root(Container) + 1 merged(Image) = 2 节点（原 3 节点，两 Image 合并）。
        // 注意 root 是 Container(tex_id=0)，两 Image 是 tex_id=1，不同 DrawState 不合。
        let mesh_count = frame.nodes.iter()
            .filter(|n| matches!(&n.payload, NodePayload::Mesh { verts, .. } if verts.len() == 8))
            .count();
        assert_eq!(mesh_count, 1, "两同 atlas Image → 1 个 8-vert merged mesh");
    }
```

Run: `cargo test -p loomgui_core build_merges_adjacent_same_drawstate_meshes`
Expected: PASS

- [ ] **Step 3: blob round-trip 测——merged 节点经 build_blob 顶点数/alpha 正确**

在 `blob.rs` 的 `tests` mod 追加（复用 blob.rs 现有 helper 风格 `mesh_node`/`frame`，参考 blob.rs:226 附近的 `mesh_node` helper）：

```rust
    /// §v1b.4：merged FrameData（transform=0、alpha=1、多 quad 拼接）经 build_blob，
    /// re-base 减 0 = 顶点保持绝对；alpha×1 = 不变。blob 列结构零改。
    #[test]
    fn merged_mesh_blob_keeps_absolute_verts_and_no_double_alpha() {
        // 构造一个 merged 节点：8 verts（2 quad）、transform=0、alpha=1。
        let merged = loomgui_core::render::node::RenderNode {
            node_id: 1, parent_id: None, visible: true, alpha: 1.0, grayed: false,
            color_tint: [1.0; 4],
            transform: loomgui_core::render::node::NodeTransform { x: 0.0, y: 0.0, ..Default::default() },
            blend: loomgui_core::render::node::BlendMode::Normal,
            mask_context: loomgui_core::render::node::MaskContext(0),
            sort_key: 0,
            payload: loomgui_core::render::node::NodePayload::Mesh {
                // 顶点已是绝对 design（merge 不 re-base）；re-base 减 transform(0) = 不变。
                verts: vec![[0.0,0.0],[10.0,0.0],[10.0,10.0],[0.0,10.0],
                            [100.0,0.0],[110.0,0.0],[110.0,10.0],[100.0,10.0]],
                uvs: vec![[0.0,0.0]; 8],
                colors: vec![[1.0,1.0,1.0,1.0]; 4, [1.0,1.0,1.0,0.5]; 4], // 第二组 alpha 已烤 0.5
                indices: vec![0,1,2,0,2,3, 4,5,6,4,6,7],
                texture: 1, program: 0,
            },
        };
        let frame = loomgui_core::render::FrameData {
            nodes: vec![merged],
            clips: vec![],
        };
        let buf = build_blob(&frame);
        let view = BlobView::new(&buf);
        assert_eq!(view.node_count(), 1);
        assert_eq!(view.payload_kind(0), 1, "merged 仍是 Mesh payload_kind=1");
        // merged 顶点 8 个，re-base 减 0 = 绝对原值。
        let (vc, _ic) = view.mesh_vert_count(0);
        assert_eq!(vc, 8, "merged segment 8 顶点");
        // 第一顶点 = (0,0) 绝对（re-base 减 0）。
        let (vx, vy) = view.mesh_vert(0, 0);
        assert_eq!((vx, vy), (0.0, 0.0));
        // 第五顶点（第二 quad 首）= (100,0) 绝对，证明未 re-base 到本地。
        let (vx5, vy5) = view.mesh_vert(0, 4);
        assert_eq!((vx5, vy5), (100.0, 0.0));
        // 第二组 colors alpha=0.5，blob 再 ×alpha(1.0)=不变。
        let ca = view.mesh_color_alpha(0, 4);
        assert!((ca - 0.5).abs() < 1e-6, "merged alpha=1 → blob 不二次烤");
    }
```

> 注：`BlobView::mesh_vert_count/mesh_vert/mesh_color_alpha` 若 blob.rs 现有 view helper 未提供逐顶点读取，implementer 须在 test-only `BlobView` 上补这三个读法（参考 blob.rs:497 附近的 `BlobView` impl，按 mesh_off/mesh_len 偏移解码 segment：seg 布局 = vert_count:u32, idx_count:u32, verts[vc×2 f32], uvs[vc×2], colors[vc×4], indices）。这是 test helper 扩展，不进生产代码。

Run: `cargo test -p loomgui_ffi_c merged_mesh_blob_keeps_absolute_verts_and_no_double_alpha`
Expected: PASS

- [ ] **Step 4: 黄金等价测——inline 包渲染不受 merge 影响（视觉等价）**

`stage.rs:93` 的 `package_load_renders_identical_to_inline` 现在两个路径都经 reorder+merge，输出仍应一致（同输入同合并）。跑该测确认不破：

Run: `cargo test -p loomgui_core --features parse stage::tests::package_load_renders_identical_to_inline`
Expected: PASS（两路径合并行为同构，JSON 一致）

> 若该测因 merge 改变 node 顺序而 FAIL：说明 inline 与 pkg 路径的节点 sort_key 输入不一致——检查两路径 scene 是否同构。正常应一致（同 build_render_nodes）。

- [ ] **Step 5: workspace 绿 + commit**

Run: `cargo test --workspace`
Expected: 全 PASS

```bash
git add loomgui_core/src/render/mod.rs loomgui_ffi_c/src/blob.rs
git commit -m "feat(v1b.4): 接入 reorder+merge 到 build_render_nodes；blob round-trip 验证零改"
```

---

### Task 4: Unity EditMode 测（merged blob fixture）

**Files:**
- Create: `loomgui_unity/Assets/LoomGUI/Tests/MergeMirrorPoolTests.cs`
- Reference: `loomgui_unity/Assets/LoomGUI/Tests/AtlasMirrorPoolTests.cs`（fixture 构造模板）、`FrameBlob.cs`（14 列布局）

**Interfaces:**
- Consumes: `FrameBlob`（解析 blob）、`MirrorPool.Sync`（消费 blob 产 GO）、`MaterialManager`
- Produces: 验证 merged blob（大 mesh segment）经 MirrorPool → 1 个 GO（而非 N 个）+ 大 Mesh 顶点数。

**关键**：手搓 merged blob fixture（参考 AtlasMirrorPoolTests 的 byte 构造法）。merged 节点 = 1 个 node_count=1 的 blob，mesh segment 含 8 顶点（2 quad 拼接）、transform=(0,0)、alpha=1。

- [ ] **Step 1: 写失败测——merged blob → 1 GO + 8 顶点**

创建 `MergeMirrorPoolTests.cs`：

```csharp
using NUnit.Framework;
using UnityEngine;
using LoomGUI;

namespace LoomGUI.Tests
{
    /// v1b.4：merged blob（1 节点、8 顶点拼接 mesh）→ MirrorPool 产 1 个 GO + 大 Mesh。
    /// 对照：v1b.3 两节点 blob → 2 个 GO。merged 让 N→1 GO（→ N→1 draw call）。
    public class MergeMirrorPoolTests
    {
        GameObject _root;
        MirrorPool _pool;

        [SetUp]
        public void SetUp()
        {
            _root = new GameObject("root");
            _pool = new MirrorPool(_root);
        }

        [TearDown]
        public void TearDown()
        {
            Object.DestroyImmediate(_root);
        }

        /// 构造一个 merged blob：1 节点，mesh segment = 8 顶点（2 quad 拼接）、6+6=12 indices、
        /// transform=(0,0)、alpha=1、tex_id=1、mask_context=0、payload_kind=1。
        /// 参考 FrameBlob.cs 14 列布局 + mesh arena segment 布局。
        byte[] BuildMergedBlob()
        {
            // implementer 照 AtlasMirrorPoolTests 的 byte 构造法填：
            // header(magic,version=3,node_count=1) + 14 col_offsets + arena offsets
            // + 14 列各 1 元素 + mesh_arena(1 segment: vc=8,ic=12, verts[8×2], uvs[8×2], colors[8×4], indices[12])
            // node_id=1, parent_id=-1, visible=1, alpha=1.0, sort_key=0, local_x=0, local_y=0,
            // mask_context=0, payload_kind=1, mesh_off/len, text_off/len=0, tex_id=1
            // 顶点绝对坐标：(0,0)(10,0)(10,10)(0,10)(100,0)(110,0)(110,10)(100,10)
            // （照搬 T3 Step3 的 Rust fixture 值，验 re-base 减 0 后 Unity 读到绝对坐标）
            throw new System.NotImplementedException("照 AtlasMirrorPoolTests 模板填完整 byte 构造");
        }

        [Test]
        public void MergedBlobProducesSingleGoWithEightVerts()
        {
            var blob = new FrameBlob(BuildMergedBlob());
            _pool.Sync(blob, _root.transform, scale: 1f);

            // merged 1 节点 → 1 个 loom_node GO（非 2）。
            var nodes = System.Array.FindAll(_root.GetComponentsInChildren<MeshRenderer>(),
                mr => mr.gameObject.name == "loom_node");
            Assert.AreEqual(1, nodes.Length, "merged blob → 1 GO（非 2）");

            // 该 GO 的 Mesh 有 8 顶点（2 quad 拼接）。
            var mf = nodes[0].GetComponent<MeshFilter>();
            Assert.AreEqual(8, mf.sharedMesh.vertexCount, "merged mesh 8 顶点");
        }
    }
}
```

- [ ] **Step 2: 填完 BuildMergedBlob 实现，跑测通过**

implementer 照 `AtlasMirrorPoolTests.cs` 的 byte 构造模板填 `BuildMergedBlob`（header + 14 列 + mesh arena segment），去掉 `NotImplementedException`。

Run: Unity EditMode → `MergeMirrorPoolTests`
Expected: PASS（1 GO + 8 顶点）

- [ ] **Step 3: 加 .meta + commit**

新 `.cs` 的 `.meta` 必入库（knowledge 坑：合 main 后 .meta 漏入库）。

```bash
git add loomgui_unity/Assets/LoomGUI/Tests/MergeMirrorPoolTests.cs loomgui_unity/Assets/LoomGUI/Tests/MergeMirrorPoolTests.cs.meta
git commit -m "test(v1b.4): Unity EditMode merged blob → 1 GO + 8 顶点"
```

---

### Task 5: sample + release .dll + PlayMode 验收（押用户）

**Files:**
- Create: `loomgui_pkg/samples/merge/{page.html,page.css}`（多 sprite 连续排列，验证 N→1）
- Reuse: `loomgui_pkg/samples/atlas/{red.png,green.png,blue.png}`（或新 3 色 PNG）
- Modify: `loomgui_unity/Assets/StreamingAssets/loom_atlas.pkg.bin`（regen，复用 v1b.3 packer 输出 v2 格式）
- Modify: `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`（重编 release，knowledge 坑10）

**验收标准**（PlayMode，押用户）：
- EditMode 全绿。
- sample 渲染：多 sprite 正确显示（视觉与 v1b.3 一致，无花屏/错位——证 merge 正确性）。
- FrameDebugger：连续同 atlas sprite 段 → draw call 数 < v1b.3（理想 1）。GO 数 = batch 数（<< sprite 数）。
- 无报错。

- [ ] **Step 1: 写 sample（html/css 多连续 sprite）**

`loomgui_pkg/samples/merge/page.html`：
```html
<div class="c"><img src="red.png"><img src="green.png"><img src="blue.png"></div>
```
`loomgui_pkg/samples/merge/page.css`：
```css
.c{display:flex;flex-direction:row;align-items:flex-start;width:600px;height:200px;gap:8px;background-color:#222222;}
```
（3 sprite 连续排列、同 atlas → reorder 让其相邻 → merge 成 1 mesh → 1 draw call。）

- [ ] **Step 2: 跑 packer 产 pkg.bin + atlas.png**

Run（在 loomgui_pkg）：`cargo run --release -- pack samples/merge/page.html samples/merge/page.css 600 200 samples/merge/ -o ../loomgui_unity/Assets/StreamingAssets/loom_atlas.pkg.bin`
（照 v1b.3 T7 的 packer 调用；复用 red/green/blue.png 或复制到 samples/merge/）

产出 `loom_atlas.pkg.bin`（v2 LPKG+version2）+ `loom.atlas.png`，commit 入 StreamingAssets。

- [ ] **Step 3: 重编 release .dll**

Run：`cargo build --release -p loomgui_ffi_c`，把 `target/release/loomgui_ffi_c.dll`（或 .lib → dll）复制到 `loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`。
（knowledge 坑10：Unity 开着锁 .dll，须关 Unity 或先停 PlayMode 再换。md5 记 ledger。）

- [ ] **Step 4: commit sample + .dll + pkg.bin**

```bash
git add loomgui_pkg/samples/merge/ loomgui_unity/Assets/StreamingAssets/loom_atlas.pkg.bin loomgui_unity/Assets/StreamingAssets/loom.atlas.png loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll
git commit -m "feat(v1b.4): merge sample + release .dll + regen pkg.bin"
```

- [ ] **Step 5: PlayMode 验收（押用户）**

用户在 Unity：
1. EditMode 跑全测（含 MergeMirrorPoolTests）→ 绿。
2. 进 PlayMode → 3 sprite 正确显示（方块，无花屏）。
3. FrameDebugger → 同 atlas sprite 段 draw call 数（对比 v1b.3 的 3 → v1b.4 应 ≤ 更少，理想 1）。
4. Hierarchy → loom_node GO 数 = batch 数（<< 3）。
5. 无报错。

用户确认 → task 完成，进 final whole-branch review（opus）。

---

## Self-Review

**1. Spec coverage:**
- §4 架构（reorder + merge + blob 不变）→ T1/T2/T3 ✓
- §5 DrawState key → T1 `draw_state`/`mesh_key`（texture,program,mask_context）✓
- §6 AABB 重排（BatchingRoot/稳定插入排序/Text break/重赋 sort_key）→ T1 ✓
- §7 merge（verts cat/colors 烤 alpha/index 偏移/transform=0/alpha=1）→ T2 ✓
- §8 锚 node_id → T2 `merge_batch`（min node_id）✓
- §9 blob/Unity 零改 → T3 blob round-trip 测 + T4 Unity 测 ✓
- §10 边界（Text/clip/texture/相交）→ T1/T2 测覆盖 ✓
- §11 测试（core 纯算法 + Unity EditMode + PlayMode）→ T1-T5 ✓
- §12 defer（YAGNI）→ 不实现，Global Constraints 声明 ✓

**2. Placeholder scan:**
- T4 Step1 `BuildMergedBlob` 用 `NotImplementedException` 占位 + Step2 填实现 —— 这是 TDD 两步（先 RED 后 GREEN），非 plan placeholder；Step2 明确要求照 AtlasMirrorPoolTests 模板填。✓
- 无 TBD/TODO/「适当处理」。

**3. Type consistency:**
- `reorder_for_batching(scene: &Scene, nodes: &mut [RenderNode])` —— T1 定义、T3 调用，签名一致 ✓
- `merge_meshes(nodes: Vec<RenderNode>) -> Vec<RenderNode>` —— T2 定义、T3 调用，签名一致 ✓
- `mesh_key`/`draw_state` 返回 tuple：T1 `draw_state` = `Option<(u32,u32)>`（texture,mask_context，program 已由 is_mergeable_mesh 保证）；T2 `mesh_key` = `Option<(u32,u32,u32)>`（texture,program,mask_context）。**两者不同**——T1 在 unit 内（已筛 mergeable），T2 在全扫描（需含 program 判 program=0）。这是有意的（职责不同），但 reviewer 可能要求统一。**决议**：保留差异（T1 unit 内 program 已定，T2 需判 program）。已在函数注释说明。✓
- 锚 node_id：T2 `merge_batch` `anchor = min(node_id)`，T4/T5 验证一致 ✓
- colors 烤制：T2 `col[3] *= alpha`，T3 blob 测验证不二次烤 ✓

**4. 风险点（reviewer 关注）:**
- T1 `reorder_for_batching` 的"重排单元识别"用 `mask_context` 相等作 BatchingRoot 边界 —— 正确性依赖 assign_sort_keys 的 mask_context 分层（同 root 子树同 ctx，子 root 开新 ctx）。与 batch.rs 现有语义一致 ✓
- T3 Step4 黄金等价测若 FAIL → 说明 merge 对两路径不同构，须排查（已在 step 注释提示）。
- T4 手搓 blob fixture 是已知 pain point（坑21）—— Step1/Step2 拆 RED/GREEN，implementer 照模板填。
