//! 命中测试：输入 design 坐标点 → 返回命中 NodeId。
//! 逆等效绘制序遍历（顶层优先），layout_rect AABB + clip 门控 + pointer-events。
//! 不做 transform world_to_local（无动画故无影响）。

use crate::scene::node::{NodeId, Rect, Scene};

/// 点是否在 Rect 内（含边界，design 坐标）。
pub(crate) fn point_in_rect(point: (f32, f32), r: Rect) -> bool {
    point.0 >= r.x && point.0 <= r.x + r.w && point.1 >= r.y && point.1 <= r.y + r.h
}

/// children 按 style.order 降序排（大值=顶层在前）；同 order 时后出现的子在前
/// （CSS flexbox `order` 语义：默认 order=0，DOM 序 = 绘制序，后者绘 = 顶层）。
/// 实现：先反转 children（让后者靠前），再按 `-order` 稳定排——stable 保反转后序，
/// 即同 order 下后者先测，与 hit_test"顶层优先"一致。
fn effective_draw_order(scene: &Scene, parent: NodeId) -> Vec<NodeId> {
    let mut kids: Vec<NodeId> = scene.nodes[parent.0 as usize].children.clone();
    kids.reverse();
    kids.sort_by_key(|&c| -scene.nodes[c.0 as usize].style.order); // 负号=降序
    kids
}

/// 命中合成 scrollbar thumb → (container_id, axis: 0=v 1=h)。None 不命中。
/// scrollbar 最上层——遍历所有容器 check v/h thumb rect。
pub fn hit_scrollbar_grip(scene: &Scene, point: (f32, f32)) -> Option<(NodeId, u8)> {
    for id in 0..scene.nodes.len() {
        let nid = NodeId(id as u32);
        if let Some(r) = crate::scroll::v_thumb_rect(scene, nid) {
            if point_in_rect(point, r) {
                return Some((nid, 0));
            }
        }
        if let Some(r) = crate::scroll::h_thumb_rect(scene, nid) {
            if point_in_rect(point, r) {
                return Some((nid, 1));
            }
        }
    }
    None
}

/// 命中测试。逆等效绘制序遍历，第一个命中即返回（顶层优先）。
/// scrollbar thumb 最上层，前置 check。
pub fn hit_test(scene: &Scene, point: (f32, f32)) -> Option<NodeId> {
    // scrollbar grip 最上层（先于所有 Scene 节点）
    if let Some((container, axis)) = hit_scrollbar_grip(scene, point) {
        let flag = if axis == 0 {
            crate::scroll::V_THUMB_FLAG
        } else {
            crate::scroll::H_THUMB_FLAG
        };
        return Some(NodeId(container.0 | flag));
    }
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
    let node = &scene.nodes[id.0 as usize];
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
    // 子都不命中 → 自身 fallback：touchable + 点经 world matrix 逆投到本地 box
    // world_to_local：点经 world matrix 逆投到本地，判本地 box (0,0,w,h)
    if node.touchable {
        let wm = scene.world_transforms[id.0 as usize];
        let inv = crate::transform::inverse(&wm);
        let (lx, ly) = crate::transform::apply_point(&inv, point.0, point.1);
        let lr = node.layout_rect;
        if lx >= 0.0 && lx <= lr.w && ly >= 0.0 && ly <= lr.h {
            return Some(id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::{Node, NodeId, NodeKind, Rect, Scene};
    use crate::scene::transform::compute_world_transforms;
    use crate::style::resolved::LocalTransform;
    use crate::transform;
    use crate::transform::Affine2Ext;

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
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        }
    }

    #[test]
    fn hit_test_returns_none_on_empty_scene() {
        let mut s = Scene {
            roots: vec![],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        compute_world_transforms(&mut s);
        assert_eq!(hit_test(&s, (10.0, 10.0)), None);
    }

    #[test]
    fn hit_test_hits_topmost_child() {
        let mut s = overlap_scene();
        compute_world_transforms(&mut s);
        // 点 (75,75) 在 a 和 b 重叠区——b 顶层（后绘制）应命中
        assert_eq!(hit_test(&s, (75.0, 75.0)), Some(NodeId(2)));
    }

    #[test]
    fn hit_test_hits_only_child_when_no_overlap() {
        let mut s = overlap_scene();
        compute_world_transforms(&mut s);
        // 点 (10,10) 只在 a 内
        assert_eq!(hit_test(&s, (10.0, 10.0)), Some(NodeId(1)));
    }

    #[test]
    fn hit_test_skips_pointer_events_none_but_tests_children() {
        let mut s = overlap_scene();
        compute_world_transforms(&mut s);
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
        compute_world_transforms(&mut s);
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
        compute_world_transforms(&mut s);
        // a 设 order=2（顶层），b order=0——等效序 a 在前
        s.nodes[1].style.order = 2;
        s.nodes[2].style.order = 0;
        // 点 (75,75) 重叠区——a 顶层应命中
        assert_eq!(hit_test(&s, (75.0, 75.0)), Some(NodeId(1)));
    }

    #[test]
    fn hit_test_disabled_node_still_target() {
        // disabled 仍参与命中（active/click 抑制在状态机层，hit_test 只返回几何命中）
        let mut s = overlap_scene();
        compute_world_transforms(&mut s);
        s.nodes[2].disabled = true; // b disabled
        // 点 (75,75) 在 b 内——b 仍命中（disabled 不跳过）
        assert_eq!(hit_test(&s, (75.0, 75.0)), Some(NodeId(2)));
    }

    #[test]
    fn hit_rotated_parent_catches_child_via_world_to_local() {
        // parent rotate(90°) at (0,0,100,100)；child identity at (0,0,10,10)。
        // parent 绕 center(50,50) 转 90°：child(在 parent 左上角) 视觉转到 parent 右上区域。
        // 命中点取 child 旋转后的中心附近。
        let mut s = overlap_scene_rotated();
        compute_world_transforms(&mut s);
        // child world == parent world（identity 子继承）。parent 旋转后 child box 在新位置。
        // 用 child box center 经 parent.world 变换得世界中心，命中应返 child。
        let child_wm = s.world_transforms[2];
        let (cx, cy) = child_wm.apply_point(5.0, 5.0); // child 本地中心
        assert_eq!(hit_test(&s, (cx, cy)), Some(NodeId(2)), "点在旋转后 child 上 → 命中 child");
    }

    fn overlap_scene_rotated() -> Scene {
        let mut root = Node::default();
        root.id = NodeId(0);
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut parent = Node::default();
        parent.id = NodeId(1); parent.parent = Some(NodeId(0));
        parent.layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        parent.style.transform = LocalTransform { matrix: transform::from_rotate(std::f32::consts::FRAC_PI_2) };
        let mut child = Node::default();
        child.id = NodeId(2); child.parent = Some(NodeId(1));
        child.layout_rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        root.children = vec![NodeId(1)];
        parent.children = vec![NodeId(2)];
        Scene { roots: vec![NodeId(0)], nodes: vec![root, parent, child],
                dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new() }
    }

    // ── hit_scrollbar_grip ─────────────────────────────────

    fn scroll_scene_with_thumb() -> Scene {
        use crate::style::resolved::{OverflowMode, ResolvedStyle};
        let mut scroll_style = ResolvedStyle::default();
        scroll_style.overflow_y = OverflowMode::Scroll;
        let entries: Vec<(
            Option<usize>,
            NodeKind,
            ResolvedStyle,
            Vec<String>,
            Option<String>,
            bool,
            Option<i32>,
        )> = vec![
            (None, NodeKind::Container, scroll_style.clone(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let mut s = Scene::build(&entries);
        s.nodes[0].layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        s.nodes[1].layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.nodes[2].layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 }; // content_y=200 > viewport=100
        crate::scroll::refresh_content_sizes(&mut s);
        compute_world_transforms(&mut s);
        s
    }

    #[test]
    fn hit_scrollbar_grip_returns_container() {
        let s = scroll_scene_with_thumb();
        // thumb 右边缘 (x=92..100, y=0..50)，取 center (96, 25)
        let result = hit_scrollbar_grip(&s, (96.0, 25.0));
        assert!(result.is_some(), "thumb 内一点应命中");
        let (container, axis) = result.unwrap();
        assert_eq!(container, NodeId(0), "返容器 NodeId(0)");
        assert_eq!(axis, 0, "垂直 thumb axis=0");
    }

    #[test]
    fn hit_scrollbar_grip_returns_none_outside_thumb() {
        let s = scroll_scene_with_thumb();
        // 点在容器左上角 (10,10) 非 thumb 区
        assert!(hit_scrollbar_grip(&s, (10.0, 10.0)).is_none(), "非 thumb 区 → None");
    }

    #[test]
    fn hit_scrollbar_grip_no_scroll_no_thumb() {
        let s = overlap_scene(); // 无 scroll 容器
        compute_world_transforms(&mut s.clone());
        assert!(hit_scrollbar_grip(&s, (50.0, 50.0)).is_none(), "无 scroll 容器 → None");
    }

    #[test]
    fn hit_test_returns_sentinel_for_thumb() {
        let s = scroll_scene_with_thumb();
        // thumb 内一点 → hit_test 应返 sentinel（含 V_THUMB_FLAG）
        let hit = hit_test(&s, (96.0, 25.0));
        assert!(hit.is_some(), "thumb 区 hit_test 命中");
        let raw = hit.unwrap().0 as u32;
        assert!(raw & crate::scroll::V_THUMB_FLAG != 0, "sentinel 含 V_THUMB_FLAG");
        // 去掉 flag 应得 container id
        assert_eq!(raw & !crate::scroll::V_THUMB_FLAG, 0u32, "flag off → container 0");
    }
}
