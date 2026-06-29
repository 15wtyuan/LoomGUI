//! FairyBatching：sort_key 分配 + 绘制序 + rect clip mask_context。
//!
//! 简化（明确不做的事，留作后续优化）：
//! - **sort_key = DFS 出现序**：单一全局计数器，自增即赋值；不做 AABB 重排合并。
//!   保序即正确意图（重排是性能优化，保序能跑通管线即可）。
//! - **mask_context**：clip_rect 的 Container 是 BatchingRoot，开新层级；
//!   子树继承。用「出现序 + 1」当层级 id（计数器+1），不维护真 stencil 层级栈。
//! - **BatchingRoot 边界**：不在 Root 处断批合（FairyGUI 真实策略按贴图/program 断，
//!   当前没有贴图集 / 多 program，断无可断；留待后续）。

use crate::render::node::{MaskContext, NodePayload, RenderNode};
use crate::scene::node::{NodeId, Rect, Scene};
use crate::render::ClipEntry;

/// AABB 交集：返回 intersected Rect；无重叠 → 零面积 `{x, y, w:0, h:0}`（x/y 取
/// max-min 处的边界值，w/h=0）。永远返回 Rect（不是 None），方便 clip 表直填。
///
/// - x = max(a.x, b.x), y = max(a.y, b.y)
/// - right = min(a.x+a.w, b.x+b.w), bottom = min(a.y+a.h, b.y+b.h)
/// - 若 right<=x 或 bottom<=y → 零面积（disjoint → empty）。
///
/// 嵌套 disjoint clip → 零面积 rect（shader safe-blank 处理）。
pub fn rect_intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.w).min(b.x + b.w);
    let bottom = (a.y + a.h).min(b.y + b.h);
    let w = (right - x).max(0.0);
    let h = (bottom - y).max(0.0);
    Rect { x, y, w, h }
}

/// 是否可合并 Mesh（program=0 + 纯平移）。Text（program=1）/ Unchanged / 非纯平移不参与重排与合并。
fn is_mergeable_mesh(rn: &RenderNode) -> bool {
    matches!(&rn.payload, NodePayload::Mesh { program, .. } if *program == 0)
        && crate::transform::is_pure_translation(&rn.world_matrix)
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

/// 一个重排单元内做 fgui 式稳定插入排序。
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

/// 给所有 RenderNode 填 sort_key + mask_context，并产 clip 表（context_id → 祖先
/// clip 链交集的绝对 design rect）。
///
/// 单遍 DFS，按 scene.roots 起遍历，DFS 树序即绘制序。`nodes` 必须与 `scene.nodes`
/// 同长且同序（由 `build_render_nodes` 保证）。返回的 `Vec<ClipEntry>` 含且仅含
/// mask_context>0 的层级（context==0 = 无 clip，不入表）。
///
/// 交集语义：进入 overflow:hidden 节点（`clip_rect.is_some()`）时，把本节点 clip 与
/// 祖先 clip 链的累乘交 (`accumulated`) 求交，得 `intersected`；新 context 记
/// `(ctx, intersected)` 入表；子树 `accumulated = intersected`。非 clipper 节点继承
/// 父 `accumulated` 不变（其 mask_context 继承父层级）。
pub fn assign_sort_keys(scene: &Scene, nodes: &mut [RenderNode]) -> Vec<ClipEntry> {
    let mut counter: u32 = 0;
    let mut clips: Vec<ClipEntry> = Vec::new();
    fn dfs(
        scene: &Scene,
        nodes: &mut [RenderNode],
        id: NodeId,
        counter: &mut u32,
        clips: &mut Vec<ClipEntry>,
        parent_mask: MaskContext,
        accumulated: Option<Rect>,
        scroll_offset: (f32, f32),
    ) {
        let node = &scene.nodes[id.0];
        // mask_context + clip 交集：本节点 clip_rect 非空 → 开新层级（计数器+1），
        // 算 own ∩ accumulated；否则继承父层级与 accumulated。
        //
        // clip rect 减祖先 scroll_offset：transform.rs 给子节点 world_matrix 注入
        // T(-祖先.scroll_pos)，故节点 world 在 (layout - scroll_offset) 空间。clip rect
        // 须同空间——否则 shader clipPos（world 含 scroll）与 _ClipBox（design 不含 scroll）
        // 错位，scroll 时 CLIPPED 节点 clipPos 超界全裁。
        let (mask, intersected_for_kids) = if let Some(own) = node.clip_rect {
            let own_scrolled = Rect {
                x: own.x - scroll_offset.0,
                y: own.y - scroll_offset.1,
                w: own.w,
                h: own.h,
            };
            let intersected = match accumulated {
                None => own_scrolled,
                Some(a) => rect_intersect(a, own_scrolled),
            };
            let ctx = *counter + 1;
            clips.push(ClipEntry { context_id: ctx, rect: intersected });
            (MaskContext(ctx), Some(intersected))
        } else {
            (parent_mask, accumulated)
        };
        {
            let rn = &mut nodes[id.0];
            rn.sort_key = *counter;
            rn.mask_context = mask;
            *counter += 1;
        }
        // 子树 scroll_offset：本节点是 scroll 容器时子吃其 scroll_pos（transform.rs 同约定）。
        // accumulated 不减 scroll——祖先 clip（如 scroll 容器 viewport）在 world 固定（容器自身
        // world 不含自己 scroll_pos），own 在 dfs 入口减 scroll_offset 转 world 空间后与之求交，
        // 即得 world 可见区。传子保持 intersected（已是 world 空间可见区）。
        let child_scroll_offset = if let Some(st) = scene.scroll.get(id) {
            (scroll_offset.0 + st.scroll_pos.0, scroll_offset.1 + st.scroll_pos.1)
        } else {
            scroll_offset
        };
        let child_accumulated = intersected_for_kids;
        // clone children 避免与 nodes 的 &mut 冲突借（scene 与 nodes 是独立借用）。
        let kids = node.children.clone();
        for c in kids {
            dfs(scene, nodes, c, counter, clips, mask, child_accumulated, child_scroll_offset);
        }
    }
    for root in &scene.roots {
        dfs(scene, nodes, *root, &mut counter, &mut clips, MaskContext(0), None, (0.0, 0.0));
    }
    clips
}

/// AABB 保序重排：按 BatchingRoot（mask_context）分段，段内对 program=0
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::node::{BlendMode, NodePayload};
    use crate::scene::node::*;

    fn placeholder_rn(i: usize) -> RenderNode {
        RenderNode {
            node_id: i as u32,
            parent_id: if i == 0 { None } else { Some(0) },
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            world_matrix: crate::transform::IDENTITY,
            blend: BlendMode::Normal,
            mask_context: MaskContext(0),
            sort_key: 0,
            payload: NodePayload::Unchanged,
        }
    }

    /// 构造 root > [a, b]，全部 Container 无 clip。
    fn tree_root_two_kids() -> Scene {
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        let mut root = Node::default();
        root.id = NodeId(0);
        root.children = vec![NodeId(1), NodeId(2)];
        scene.nodes.push(root);

        let mut a = Node::default();
        a.id = NodeId(1);
        a.parent = Some(NodeId(0));
        scene.nodes.push(a);

        let mut b = Node::default();
        b.id = NodeId(2);
        b.parent = Some(NodeId(0));
        scene.nodes.push(b);
        scene
    }

    #[test]
    fn keys_monotonic() {
        let scene = tree_root_two_kids();
        let mut rns: Vec<RenderNode> = (0..3).map(placeholder_rn).collect();
        assign_sort_keys(&scene, &mut rns);
        // DFS 树序：root(0) → a(1) → b(2)
        assert!(rns[0].sort_key < rns[1].sort_key);
        assert!(rns[1].sort_key < rns[2].sort_key);
        assert_eq!(rns[0].sort_key, 0);
        assert_eq!(rns[1].sort_key, 1);
        assert_eq!(rns[2].sort_key, 2);
    }

    #[test]
    fn no_clip_keeps_mask_zero() {
        let scene = tree_root_two_kids();
        let mut rns: Vec<RenderNode> = (0..3).map(placeholder_rn).collect();
        assign_sort_keys(&scene, &mut rns);
        for rn in &rns {
            assert_eq!(rn.mask_context, MaskContext(0), "无 clip 应保持 mask=0");
        }
    }

    #[test]
    fn clip_node_opens_new_mask_layer() {
        // root(clip) > child：root 开新 mask 层，child 继承。
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        let mut root = Node::default();
        root.id = NodeId(0);
        root.clip_rect = Some(Rect::default()); // 开 clip
        root.children = vec![NodeId(1)];
        scene.nodes.push(root);

        let mut child = Node::default();
        child.id = NodeId(1);
        child.parent = Some(NodeId(0));
        scene.nodes.push(child);

        let mut rns: Vec<RenderNode> = (0..2).map(placeholder_rn).collect();
        assign_sort_keys(&scene, &mut rns);
        // root 是首个分配（counter=0），clip → MaskContext(0+1)=1
        assert_eq!(rns[0].mask_context, MaskContext(1), "clip root 开层级 1");
        assert_eq!(rns[1].mask_context, MaskContext(1), "child 继承父层级");
    }

    #[test]
    fn nested_clip_opens_distinct_layers() {
        // root(clip) > mid(clip) > leaf：root=层1，mid=层N（N>1），leaf=mid 层。
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        let mut root = Node::default();
        root.id = NodeId(0);
        root.clip_rect = Some(Rect::default());
        root.children = vec![NodeId(1)];
        scene.nodes.push(root);

        let mut mid = Node::default();
        mid.id = NodeId(1);
        mid.parent = Some(NodeId(0));
        mid.clip_rect = Some(Rect::default());
        mid.children = vec![NodeId(2)];
        scene.nodes.push(mid);

        let mut leaf = Node::default();
        leaf.id = NodeId(2);
        leaf.parent = Some(NodeId(1));
        scene.nodes.push(leaf);

        let mut rns: Vec<RenderNode> = (0..3).map(placeholder_rn).collect();
        let _clips = assign_sort_keys(&scene, &mut rns);
        // root: counter=0 → mask(1)
        // mid:  counter=1 → mask(2)
        // leaf: counter=2 → 继承 mid mask(2)
        assert_eq!(rns[0].mask_context, MaskContext(1));
        assert_eq!(rns[1].mask_context, MaskContext(2));
        assert_eq!(rns[2].mask_context, MaskContext(2));
    }

    #[test]
    fn clip_rect_in_scroll_container_is_scroll_adjusted() {
        // scroll 容器（scroll_pos 非零）+ 子 overflow:hidden。
        // 子的 clip rect 必须减祖先 scroll offset（= layout - scroll_pos），与子节点 world_matrix
        // （transform.rs 注入 T(-scroll_pos)）同空间——否则 shader clipPos（world 含 scroll）与
        // _ClipBox（design 不含 scroll）错位 → scroll 时 CLIPPED 节点 clipPos 超界全裁
        // （showcase 3.6/3.7 bg-demo/br-demo 内容空根因）。
        let mut scene = Scene {
            roots: vec![NodeId(0)], nodes: vec![],
            dynamic_rules: Default::default(), focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(),
            scroll: Default::default(), text_layouts: Vec::new(),
        };
        // root(overflow:scroll, scroll_pos=(0,30)) > child(10,10,80,80 overflow:hidden)
        let mut root = Node::default();
        root.id = NodeId(0);
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        root.clip_rect = Some(Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 });
        root.children = vec![NodeId(1)];
        scene.nodes.push(root);
        let mut child = Node::default();
        child.id = NodeId(1);
        child.parent = Some(NodeId(0));
        child.layout_rect = Rect { x: 10.0, y: 10.0, w: 80.0, h: 80.0 };
        child.clip_rect = Some(Rect { x: 10.0, y: 10.0, w: 80.0, h: 80.0 });
        scene.nodes.push(child);
        scene.scroll.ensure(NodeId(0)).scroll_pos = (0.0, 30.0);

        let mut rns: Vec<RenderNode> = (0..2).map(placeholder_rn).collect();
        let clips = assign_sort_keys(&scene, &mut rns);

        // root ctx(1)：容器自身 world 不含自己 scroll_pos（transform.rs 约定）→ clip rect 不减。
        let root_ctx = clips.iter().find(|c| c.context_id == 1).expect("root clip ctx");
        assert!((root_ctx.rect.y - 0.0).abs() < 1e-3, "root clip rect 不减自己 scroll_pos");
        // child ctx(2)：child world rect = (10,10-30,80,80) = (10,-20,80,80)（滚出 root viewport
        // 顶部）。可见区 = root viewport(0,0,200,200) ∩ child world(10,-20,80,80) = (10,0,80,60)。
        // clip rect 存 world 可见区（accumulated=viewport 不减 scroll；own 减 scroll_offset 转 world）。
        let child_ctx = clips.iter().find(|c| c.context_id == 2).expect("child clip ctx");
        assert!((child_ctx.rect.x - 10.0).abs() < 1e-3, "child clip x=10");
        assert!((child_ctx.rect.y - 0.0).abs() < 1e-3,
            "child clip y=0（world 可见区顶，被 root viewport 裁），得 {}", child_ctx.rect.y);
        assert!((child_ctx.rect.h - 60.0).abs() < 1e-3,
            "child clip h=60（80−滚出的 20），得 {}", child_ctx.rect.h);
    }

    // —— 嵌套 clip 交集（rect mask）——
    // DFS 算祖先 clip 链交集，clip 表存 intersected rect（否则 leaf 的 clip rect
    // 只等于最内层 clipper 的 box，外层 disjoint clip 泄漏）。

    /// nested disjoint: outer [0,0,100,100] > inner [200,200,50,50]（不相交）> leaf。
    /// inner 的 context 对应 clip rect 必须是零面积（交集空），不是 [200,200,50,50]。
    #[test]
    fn nested_disjoint_clip_intersection_is_zero_area() {
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        let mut outer = Node::default();
        outer.id = NodeId(0);
        outer.clip_rect = Some(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });
        outer.children = vec![NodeId(1)];
        scene.nodes.push(outer);

        let mut inner = Node::default();
        inner.id = NodeId(1);
        inner.parent = Some(NodeId(0));
        inner.clip_rect = Some(Rect { x: 200.0, y: 200.0, w: 50.0, h: 50.0 });
        inner.children = vec![NodeId(2)];
        scene.nodes.push(inner);

        let mut leaf = Node::default();
        leaf.id = NodeId(2);
        leaf.parent = Some(NodeId(1));
        scene.nodes.push(leaf);

        let mut rns: Vec<RenderNode> = (0..3).map(placeholder_rn).collect();
        let clips = assign_sort_keys(&scene, &mut rns);

        // mask_context: outer=1, inner=2, leaf 继承 inner=2。
        assert_eq!(rns[0].mask_context, MaskContext(1));
        assert_eq!(rns[1].mask_context, MaskContext(2));
        assert_eq!(rns[2].mask_context, MaskContext(2));

        // outer context(1) 的 clip rect == outer box 本身（无祖先 clip）。
        let ctx1 = clips.iter().find(|c| c.context_id == 1).expect("ctx 1 in table");
        assert_eq!((ctx1.rect.x, ctx1.rect.y, ctx1.rect.w, ctx1.rect.h),
                   (0.0, 0.0, 100.0, 100.0));

        // inner context(2) 的 clip rect == outer ∩ inner = 零面积（不相交）。
        let ctx2 = clips.iter().find(|c| c.context_id == 2).expect("ctx 2 in table");
        assert_eq!(ctx2.rect.w, 0.0, "disjoint 交集 w=0");
        assert_eq!(ctx2.rect.h, 0.0, "disjoint 交集 h=0");
        // 关键断言：不是 [200,200,50,50]（只裁最内层会泄漏外层 disjoint clip）。
        assert!(!(ctx2.rect.x == 200.0 && ctx2.rect.y == 200.0
                  && ctx2.rect.w == 50.0 && ctx2.rect.h == 50.0),
                "inner context rect 不应等于 inner box（只裁最内层会泄漏）");
    }

    /// nested overlapping: outer [0,0,100,100] > inner [50,50,100,100]（重叠）> leaf。
    /// inner context rect == 交集 [50,50,50,50]。
    #[test]
    fn nested_overlapping_clip_intersection_is_overlap_rect() {
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        let mut outer = Node::default();
        outer.id = NodeId(0);
        outer.clip_rect = Some(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });
        outer.children = vec![NodeId(1)];
        scene.nodes.push(outer);

        let mut inner = Node::default();
        inner.id = NodeId(1);
        inner.parent = Some(NodeId(0));
        inner.clip_rect = Some(Rect { x: 50.0, y: 50.0, w: 100.0, h: 100.0 });
        inner.children = vec![NodeId(2)];
        scene.nodes.push(inner);

        let mut leaf = Node::default();
        leaf.id = NodeId(2);
        leaf.parent = Some(NodeId(1));
        scene.nodes.push(leaf);

        let mut rns: Vec<RenderNode> = (0..3).map(placeholder_rn).collect();
        let clips = assign_sort_keys(&scene, &mut rns);

        let ctx2 = clips.iter().find(|c| c.context_id == 2).expect("ctx 2 in table");
        // outer ∩ inner = [max(0,50), max(0,50)] .. [min(100,150), min(100,150)]
        //                = [50,50,50,50]
        assert_eq!((ctx2.rect.x, ctx2.rect.y, ctx2.rect.w, ctx2.rect.h),
                   (50.0, 50.0, 50.0, 50.0),
                   "overlapping 交集 = [50,50,50,50]");
    }

    // —— AABB 保序重排（reorder_unit / reorder_for_batching）——
    // NodePayload / MaskContext / BlendMode / Node / NodeKind / Rect / Scene
    // 已由上方 use 语句导入；以下测直接使用。

    /// 构造 program=0 Mesh RenderNode（给 reorder_unit 直接喂 unit 索引对应的 nodes）。
    fn mesh_rn(tex: u32, rect: Rect, mask: u32) -> RenderNode {
        RenderNode {
            node_id: 0,
            parent_id: None,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            world_matrix: crate::transform::IDENTITY,
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
        let mut scene = Scene { roots: vec![], nodes: vec![], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new() };
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
        // A(tex1) B(tex2) C(tex1)，A 与 C AABB 相交 → C 仍前移到 A 旁（k=A 之后），
        // 但不越过 A（保 A→C 绘制序，防遮挡）。B(tex2) 被推后。
        // 注：fgui DoFairyBatching 语义非「相交=不动」，而是「向后扫到首个相交即停，
        // 但 k 已在相交前按同 material 聚拢点算出」——同 material 相交仍聚拢到紧邻。
        let mut scene = Scene { roots: vec![], nodes: vec![], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new() };
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
        // C 同 tex1 聚拢到 A 旁（k=A 之后=1），不越 A（保 A→C 序）；B 被推后。
        assert_eq!(unit, vec![0, 2, 1], "同 DrawState 相交 → 聚拢到紧邻，不越目标");
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

    #[test]
    fn reorder_splits_at_text_break() {
        // root > [A(tex1), Text, B(tex1)]：AABB 全不相交。Text 断单元 →
        // A、B 分属两个单元，B 不能跨 Text 前移到 A 旁（保 Text 绘制序）。
        let mut scene = Scene { roots: vec![NodeId(0)], nodes: vec![], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new() };
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
        let mut scene = Scene { roots: vec![NodeId(0)], nodes: vec![], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new() };
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
}
