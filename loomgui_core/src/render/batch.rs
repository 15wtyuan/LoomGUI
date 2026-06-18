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
use crate::scene::node::{NodeId, Scene};

/// 给所有 RenderNode 填 sort_key + mask_context。
///
/// 单遍 DFS，按 scene.roots 起遍历，DFS 树序即绘制序。`nodes` 必须与 `scene.nodes`
/// 同长且同序（由 `build_render_nodes` 保证）。
pub fn assign_sort_keys(scene: &Scene, nodes: &mut [RenderNode]) {
    let mut counter: u32 = 0;
    fn dfs(
        scene: &Scene,
        nodes: &mut [RenderNode],
        id: NodeId,
        counter: &mut u32,
        parent_mask: MaskContext,
    ) {
        let node = &scene.nodes[id.0];
        // mask_context：本节点 clip_rect 非空 → 开新层级（v0 简化：层级 = 计数器+1）。
        // 否则继承父层级。
        let mask = if node.clip_rect.is_some() {
            MaskContext(*counter + 1)
        } else {
            parent_mask
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
            dfs(scene, nodes, c, counter, mask);
        }
    }
    for root in &scene.roots {
        dfs(scene, nodes, *root, &mut counter, MaskContext(0));
    }
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
        assign_sort_keys(&scene, &mut rns);
        // root: counter=0 → mask(1)
        // mid:  counter=1 → mask(2)
        // leaf: counter=2 → 继承 mid mask(2)
        assert_eq!(rns[0].mask_context, MaskContext(1));
        assert_eq!(rns[1].mask_context, MaskContext(2));
        assert_eq!(rns[2].mask_context, MaskContext(2));
    }
}
