//! FairyBatching（§8.5 / §8.8）：sort_key 分配 + 绘制序 + rect clip mask_context。
//!
//! v0 简化（明确不做的事，留作 v1.x 优化）：
//! - **sort_key = DFS 出现序**：单一全局计数器，自增即赋值；不做 AABB 重排合并。
//!   保序即正确意图（重排是性能优化，v0 保序能跑通管线即可）。
//! - **mask_context**：clip_rect 的 Container 是 BatchingRoot，开新层级；
//!   子树继承。v0 用「出现序 + 1」当层级 id（计数器+1），不维护真 stencil 层级栈。
//! - **BatchingRoot 边界**：v0 不在 Root 处断批合（FairyGUI 真实策略按贴图/program 断，
//!   v0 没有贴图集 / 多 program，断无可断；留待 v1.x）。

use crate::render::node::{MaskContext, RenderNode};
use crate::scene::node::{NodeId, Rect, Scene};
use crate::render::ClipEntry;

/// AABB 交集：返回 intersected Rect；无重叠 → 零面积 `{x, y, w:0, h:0}`（x/y 取
/// max-min 处的边界值，w/h=0）。永远返回 Rect（不是 None），方便 clip 表直填。
///
/// - x = max(a.x, b.x), y = max(a.y, b.y)
/// - right = min(a.x+a.w, b.x+b.w), bottom = min(a.y+a.h, b.y+b.h)
/// - 若 right<=x 或 bottom<=y → 零面积（disjoint → empty）。
///
/// fgui-faithful：嵌套 disjoint clip → 零面积 rect（T6 shader safe-blank 处理）。
pub fn rect_intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.w).min(b.x + b.w);
    let bottom = (a.y + a.h).min(b.y + b.h);
    let w = (right - x).max(0.0);
    let h = (bottom - y).max(0.0);
    Rect { x, y, w, h }
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
/// 父 `accumulated` 不变（其 mask_context 继承父层级）。修 v0「只裁最内层」bug。
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
    ) {
        let node = &scene.nodes[id.0];
        // mask_context + clip 交集：本节点 clip_rect 非空 → 开新层级（计数器+1），
        // 算 own ∩ accumulated；否则继承父层级与 accumulated。
        let (mask, child_accumulated) = if let Some(own) = node.clip_rect {
            let intersected = match accumulated {
                None => own,
                Some(a) => rect_intersect(a, own),
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
        // clone children 避免与 nodes 的 &mut 冲突借（scene 与 nodes 是独立借用）。
        let kids = node.children.clone();
        for c in kids {
            dfs(scene, nodes, c, counter, clips, mask, child_accumulated);
        }
    }
    for root in &scene.roots {
        dfs(scene, nodes, *root, &mut counter, &mut clips, MaskContext(0), None);
    }
    clips
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::node::{BlendMode, NodePayload, NodeTransform};
    use crate::scene::node::*;

    fn placeholder_rn(i: usize) -> RenderNode {
        RenderNode {
            node_id: i as u32,
            parent_id: if i == 0 { None } else { Some(0) },
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            transform: NodeTransform::default(),
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

    // —— T5：嵌套 clip 交集（rect mask）——
    // v0 bug：只赋新 context 不交，leaf 的 clip rect 等于最内层 clipper（mid）的 box，
    // 外层 disjoint clip 泄漏。T5 修：DFS 算祖先 clip 链交集，clip 表存 intersected rect。

    /// nested disjoint: outer [0,0,100,100] > inner [200,200,50,50]（不相交）> leaf。
    /// inner 的 context 对应 clip rect 必须是零面积（交集空），不是 [200,200,50,50]。
    #[test]
    fn nested_disjoint_clip_intersection_is_zero_area() {
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
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
        // 关键断言：不是 v0 的 [200,200,50,50]（只裁最内层）。
        assert!(!(ctx2.rect.x == 200.0 && ctx2.rect.y == 200.0
                  && ctx2.rect.w == 50.0 && ctx2.rect.h == 50.0),
                "inner context rect 不应等于 inner box（v0 只裁最内层 bug）");
    }

    /// nested overlapping: outer [0,0,100,100] > inner [50,50,100,100]（重叠）> leaf。
    /// inner context rect == 交集 [50,50,50,50]。
    #[test]
    fn nested_overlapping_clip_intersection_is_overlap_rect() {
        let mut scene = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![],
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
}
