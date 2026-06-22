//! 命中测试（§10.1）：输入 design 坐标点 → 返回命中 NodeId。
//! 逆等效绘制序遍历（顶层优先），layout_rect AABB + clip 门控 + pointer-events。
//! v1c.1 不做 transform world_to_local（defer v1d，无动画故无影响）。

use crate::scene::node::{NodeId, Rect, Scene};

/// 点是否在 Rect 内（含边界，design 坐标）。
fn point_in_rect(point: (f32, f32), r: Rect) -> bool {
    point.0 >= r.x && point.0 <= r.x + r.w && point.1 >= r.y && point.1 <= r.y + r.h
}

/// children 按 style.order 降序排（大值=顶层在前）；同 order 时后出现的子在前
/// （CSS flexbox `order` 语义：默认 order=0，DOM 序 = 绘制序，后者绘 = 顶层）。
/// 实现：先反转 children（让后者靠前），再按 `-order` 稳定排——stable 保反转后序，
/// 即同 order 下后者先测，与 hit_test"顶层优先"一致。
fn effective_draw_order(scene: &Scene, parent: NodeId) -> Vec<NodeId> {
    let mut kids: Vec<NodeId> = scene.nodes[parent.0].children.clone();
    kids.reverse();
    kids.sort_by_key(|&c| -scene.nodes[c.0].style.order); // 负号=降序
    kids
}

/// §10.1 命中测试。逆等效绘制序遍历，第一个命中即返回（顶层优先）。
pub fn hit_test(scene: &Scene, point: (f32, f32)) -> Option<NodeId> {
    // 从 roots 逐棵 DFS。多个 root 按顺序，后 root 顶层（与渲染序一致）。
    for &root in &scene.roots {
        if let Some(hit) = hit_subtree(scene, root, point) {
            return Some(hit);
        }
    }
    None
}

/// 递归测某子树。先测子（逆等效序，顶层先），子命中返回子的；子都不命中→自身 fallback。
fn hit_subtree(scene: &Scene, id: NodeId, point: (f32, f32)) -> Option<NodeId> {
    let node = &scene.nodes[id.0];
    // clip 门控：有 clip_rect 且点不在 clip 内 → 整个子树不命中
    if let Some(clip) = node.clip_rect {
        if !point_in_rect(point, clip) {
            return None;
        }
    }
    // 先测子（逆等效绘制序 = 顶层先）
    for &c in &effective_draw_order(scene, id) {
        if let Some(hit) = hit_subtree(scene, c, point) {
            return Some(hit);
        }
    }
    // 子都不命中 → 自身 fallback：touchable + AABB 含点
    if node.touchable && point_in_rect(point, node.layout_rect) {
        return Some(id);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::{Node, NodeId, Rect, Scene};

    /// 构造两兄弟子节点的 scene：root + child_a + child_b，都 100x100，
    /// child_a 在 (0,0)，child_b 在 (50,50)（与 a 重叠右下角）。
    /// children 顺序 [a, b] → 等效序 b 顶层（后绘制）。
    fn overlap_scene() -> Scene {
        let mut root = Node::default();
        root.id = NodeId(0);
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut a = Node::default();
        a.id = NodeId(1);
        a.parent = Some(NodeId(0));
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut b = Node::default();
        b.id = NodeId(2);
        b.parent = Some(NodeId(0));
        b.layout_rect = Rect { x: 50.0, y: 50.0, w: 100.0, h: 100.0 };
        root.children = vec![NodeId(1), NodeId(2)];
        Scene {
            roots: vec![NodeId(0)],
            nodes: vec![root, a, b],
            dynamic_rules: Default::default(),
        }
    }

    #[test]
    fn hit_test_returns_none_on_empty_scene() {
        let s = Scene {
            roots: vec![],
            nodes: vec![],
            dynamic_rules: Default::default(),
        };
        assert_eq!(hit_test(&s, (10.0, 10.0)), None);
    }

    #[test]
    fn hit_test_hits_topmost_child() {
        let s = overlap_scene();
        // 点 (75,75) 在 a 和 b 重叠区——b 顶层（后绘制）应命中
        assert_eq!(hit_test(&s, (75.0, 75.0)), Some(NodeId(2)));
    }

    #[test]
    fn hit_test_hits_only_child_when_no_overlap() {
        let s = overlap_scene();
        // 点 (10,10) 只在 a 内
        assert_eq!(hit_test(&s, (10.0, 10.0)), Some(NodeId(1)));
    }

    #[test]
    fn hit_test_skips_pointer_events_none_but_tests_children() {
        let mut s = overlap_scene();
        // root touchable=false，但子 a 仍应命中（CSS 语义：none 不挡子）
        s.nodes[0].touchable = false;
        // 点 (10,10) 在 a 内——root 不命中但子 a 命中
        assert_eq!(hit_test(&s, (10.0, 10.0)), Some(NodeId(1)));
        // 点 (160,160) 在 root AABB 但不在 a/b（a=[0,100], b=[50,150]）
        // ——root touchable=false → None
        assert_eq!(hit_test(&s, (160.0, 160.0)), None);
    }

    #[test]
    fn hit_test_clip_rect_excludes_subtree() {
        let mut s = overlap_scene();
        // root 加 clip_rect (0,0,80,80)——点 (90,90) 在 root AABB 但 clip 外
        s.nodes[0].clip_rect = Some(Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 });
        // 点 (90,90) 在 b 的 AABB (50,50,100,100) 但在 root clip 外 → 子树不命中
        assert_eq!(hit_test(&s, (90.0, 90.0)), None);
        // 点 (70,70) 在 clip 内 + 在 b 内 → 命中 b
        assert_eq!(hit_test(&s, (70.0, 70.0)), Some(NodeId(2)));
    }

    #[test]
    fn hit_test_respects_order() {
        let mut s = overlap_scene();
        // a 设 order=2（顶层），b order=0——等效序 a 在前
        s.nodes[1].style.order = 2;
        s.nodes[2].style.order = 0;
        // 点 (75,75) 重叠区——a 顶层应命中
        assert_eq!(hit_test(&s, (75.0, 75.0)), Some(NodeId(1)));
    }

    #[test]
    fn hit_test_disabled_node_still_target() {
        // §4.4：disabled 仍参与命中（active/click 抑制在状态机层，hit_test 只返回几何命中）
        let mut s = overlap_scene();
        s.nodes[2].disabled = true; // b disabled
        // 点 (75,75) 在 b 内——b 仍命中（disabled 不跳过）
        assert_eq!(hit_test(&s, (75.0, 75.0)), Some(NodeId(2)));
    }
}
