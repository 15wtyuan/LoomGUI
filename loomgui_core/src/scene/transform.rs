//! 每帧 DFS 算每节点累计世界矩阵。
//! pivot = box center（w/2,h/2）；local = T(rel) ∘ T(pivot) ∘ transform ∘ T(-pivot)；
//! world = parent.world ∘ local。

use crate::scene::node::{AnimTable, NodeId, Scene};
use crate::transform::{self, Affine2};

pub fn compute_world_transforms(scene: &mut Scene) {
    let n = scene.nodes.len();
    // worlds 1 基索引（id.index()，slotmap idx 1..=N），len=N+1，idx 0 占位。
    let mut worlds: Vec<Affine2> = vec![transform::IDENTITY; n + 1];
    let roots = scene.roots.clone();
    for root in roots {
        rec(scene, &scene.anim, root, transform::IDENTITY, &mut worlds);
    }
    scene.world_transforms = worlds;
}

fn rec(scene: &Scene, anim: &AnimTable, id: NodeId, parent_world: Affine2, worlds: &mut [Affine2]) {
    let node = scene.get(id).expect("live node");
    let lr = node.layout_rect;
    let pivot = (lr.w / 2.0, lr.h / 2.0);
    let rel = match node.parent {
        Some(p) => {
            let plr = scene.get(p).expect("live parent").layout_rect;
            (lr.x - plr.x, lr.y - plr.y)
        }
        None => (lr.x, lr.y),
    };
    // transform 矩阵源 = anim.transform override（replace-override）unwrap css matrix。
    let m = anim
        .get(id)
        .and_then(|a| a.transform)
        .unwrap_or(node.style.transform.matrix);
    // local = T(rel) ∘ T(pivot) ∘ m ∘ T(-pivot)（free fn by ref，更显式）
    let local = transform::mul(
        &transform::from_translate(rel.0, rel.1),
        &transform::mul(
            &transform::from_translate(pivot.0, pivot.1),
            &transform::mul(&m, &transform::from_translate(-pivot.0, -pivot.1)),
        ),
    );
    // 父若是滚动容器，world 前乘 T(-父.scroll_pos)（offset 注入）。
    // 容器自身 world 不含自己 scroll_pos（其 world 用它父的 offset）；后代每层累积。
    // scroll_pos=(0,0) → T(0,0)=identity → no-op。
    let world = match node.parent.and_then(|p| scene.scroll.get(p)) {
        Some(st) => transform::mul(
            &parent_world,
            &transform::mul(&transform::from_translate(-st.scroll_pos.0, -st.scroll_pos.1), &local),
        ),
        None => transform::mul(&parent_world, &local),
    };
    worlds[id.index()] = world;
    let kids = node.children.clone();
    for c in kids {
        rec(scene, anim, c, world, worlds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::{Node, Rect, Scene};
    use crate::style::resolved::{LocalTransform, OverflowMode};
    use crate::transform::Affine2Ext;

    fn node(id: usize, parent: Option<usize>, rect: Rect) -> Node {
        let mut n = Node::default();
        n.id = NodeId(id as u32); // from_nodes 覆写；仅占位
        n.parent = parent.map(|p| NodeId(p as u32)); // from_nodes 覆写；仅用于 scene_with 推 edges
        n.layout_rect = rect;
        n
    }

    // scroll 容器 helper——overflow_y=Scroll（仅样式标记，scroll 表项由 ensure 注入）。
    fn scroll_node(id: usize, parent: Option<usize>, rect: Rect) -> Node {
        let mut n = node(id, parent, rect);
        n.style.overflow_y = OverflowMode::Scroll;
        n
    }

    // 从 Vec<Node> 建 Scene（parent 字段推 edges）。替代旧 nodes: vec![...] 字面量。
    fn scene_with(nodes: Vec<Node>) -> Scene {
        let edges: Vec<(usize, usize)> = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.parent.map(|p| (p.0 as usize, i)))
            .filter(|(p, _)| *p < nodes.len())
            .collect();
        Scene::from_nodes(nodes, edges)
    }

    fn root_id(s: &Scene) -> NodeId { s.roots[0] }
    fn child_id(s: &Scene, parent: NodeId, idx: usize) -> NodeId {
        s.get(parent).unwrap().children[idx]
    }

    #[test]
    fn identity_nodes_world_is_translation_to_layout() {
        // parent identity at (10,20,100,100)；child identity at (15,25,...)（rel 5,5）
        let mut s = scene_with(vec![
            node(0, None, Rect { x: 10.0, y: 20.0, w: 100.0, h: 100.0 }),
            { let c = node(1, Some(0), Rect { x: 15.0, y: 25.0, w: 10.0, h: 10.0 }); c },
        ]);
        let rid = root_id(&s);
        let cid = child_id(&s, rid, 0);
        compute_world_transforms(&mut s);
        // child world = T(10,20) ∘ T(5,5) = T(15,25)；纯平移，apply(0,0)=(15,25)
        let (x, y) = s.world_transforms[cid.index()].apply_point(0.0, 0.0);
        assert!((x - 15.0).abs() < 1e-4 && (y - 25.0).abs() < 1e-4);
        assert!(s.world_transforms[cid.index()].is_pure_translation(), "identity 子树 world 纯平移");
    }

    #[test]
    fn child_inherits_parent_rotation() {
        // parent rotate(90°) at (0,0,100,100)；child identity at (0,0,10,10)
        let mut s = scene_with(vec![
            node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }),
            node(1, Some(0), Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }),
        ]);
        let rid = root_id(&s);
        let cid = child_id(&s, rid, 0);
        s.get_mut(rid).unwrap().style.transform = LocalTransform { matrix: transform::from_rotate(std::f32::consts::FRAC_PI_2) };
        compute_world_transforms(&mut s);
        // child world == parent world（child identity local=identity）
        assert_eq!(s.world_transforms[cid.index()], s.world_transforms[rid.index()], "identity 子继承父 world");
        assert!(!s.world_transforms[rid.index()].is_pure_translation(), "父旋转 → world 非纯平移");
    }

    #[test]
    fn skew_composite_propagates_to_child_world() {
        // parent scale(2,1) at (0,0,100,100)；child rotate(45°) → child world 含剪切
        let mut s = scene_with(vec![
            node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }),
            node(1, Some(0), Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }),
        ]);
        let rid = root_id(&s);
        let cid = child_id(&s, rid, 0);
        s.get_mut(rid).unwrap().style.transform = LocalTransform { matrix: transform::from_scale(2.0, 1.0) };
        s.get_mut(cid).unwrap().style.transform = LocalTransform { matrix: transform::from_rotate(std::f32::consts::FRAC_PI_4) };
        compute_world_transforms(&mut s);
        assert!(!s.world_transforms[cid.index()].is_pure_translation(), "父非均匀缩放+子旋转 → world 剪切（非纯平移）");
    }

    #[test]
    fn translate_stacks_on_layout_rel() {
        // parent identity at (0,0,100,100)；child translate(5,0) layout at (10,0)（rel 10,0）
        // child world.apply(0,0) = (15,0)（rel 10 + translate 5）
        let mut s = scene_with(vec![
            node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }),
            node(1, Some(0), Rect { x: 10.0, y: 0.0, w: 10.0, h: 10.0 }),
        ]);
        let rid = root_id(&s);
        let cid = child_id(&s, rid, 0);
        s.get_mut(cid).unwrap().style.transform = LocalTransform { matrix: transform::from_translate(5.0, 0.0) };
        compute_world_transforms(&mut s);
        let (x, y) = s.world_transforms[cid.index()].apply_point(0.0, 0.0);
        assert!((x - 15.0).abs() < 1e-4 && y.abs() < 1e-4, "translate 叠 layout rel 不双计：rel(10)+t(5)=15");
    }

    #[test]
    fn anim_transform_override_replaces_css_matrix() {
        // node CSS transform=identity；anim.transform=scale(2,2) → world 非纯平移（吃 override）
        let mut s = scene_with(vec![ node(0, None, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }) ]);
        let rid = root_id(&s);
        let anim = s.anim.ensure(rid.index() + 1);
        anim[rid.index()].transform = Some(transform::from_scale(2.0, 2.0));
        compute_world_transforms(&mut s);
        assert!(!s.world_transforms[rid.index()].is_pure_translation(), "anim.transform override 生效（scale）");
    }

    #[test]
    fn no_anim_falls_back_to_css_matrix_zero_regression() {
        // anim 全 None → world == CSS（identity）纯平移到 layout 原点
        let mut s = scene_with(vec![ node(0, None, Rect { x: 5.0, y: 7.0, w: 10.0, h: 10.0 }) ]);
        let rid = root_id(&s);
        // 不设 anim（全 None）
        compute_world_transforms(&mut s);
        let (x, y) = s.world_transforms[rid.index()].apply_point(0.0, 0.0);
        assert!((x - 5.0).abs() < 1e-4 && (y - 7.0).abs() < 1e-4, "无 anim → CSS identity 纯平移");
        assert!(s.world_transforms[rid.index()].is_pure_translation());
    }

    // offset 注入——父 scroll_pos 影响 子 world，不影响容器自身。
    #[test]
    fn scroll_offset_applies_to_children_not_container() {
        // 容器 (0,0,100,100) overflow:scroll；子 (0,0,20,20)；scroll_pos=(0,30)
        let mut s = scene_with(vec![
            scroll_node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }),
            node(1, Some(0), Rect { x: 0.0, y: 0.0, w: 20.0, h: 20.0 }),
        ]);
        let rid = root_id(&s);
        let cid = child_id(&s, rid, 0);
        s.scroll.ensure(rid).scroll_pos = (0.0, 30.0);
        compute_world_transforms(&mut s);
        // 容器自身 world.apply(0,0) = (0,0)（容器 world 不含自己 scroll_pos）
        let (cx, cy) = s.world_transforms[rid.index()].apply_point(0.0, 0.0);
        assert!(cx.abs() < 1e-3 && cy.abs() < 1e-3, "容器自身 world 不吃自己 scroll_pos");
        // 子 world.apply(0,0) = (0, -30)（含 T(-scroll_pos)）
        let (x, y) = s.world_transforms[cid.index()].apply_point(0.0, 0.0);
        assert!(x.abs() < 1e-3 && (y - (-30.0)).abs() < 1e-3, "子吃父 scroll offset：(0,30)→(0,-30)");
    }

    #[test]
    fn scroll_pos_zero_is_no_op_zero_regression() {
        // scroll_pos=(0,0) → world 与无 scroll 表项等价（零回归）
        let nodes = vec![
            node(0, None, Rect { x: 10.0, y: 20.0, w: 100.0, h: 100.0 }),
            node(1, Some(0), Rect { x: 15.0, y: 25.0, w: 10.0, h: 10.0 }),
        ];
        let mut a = scene_with(nodes.clone());
        let mut b = scene_with(nodes);
        a.scroll.ensure(root_id(&a)); // scroll_pos=(0,0)
        compute_world_transforms(&mut a);
        compute_world_transforms(&mut b);
        assert_eq!(a.world_transforms, b.world_transforms, "scroll_pos=0 no-op：与无 scroll 表项等价");
    }

    // 嵌套 scroll 累积——3 层 scroll 容器，offset 逐层叠加。
    #[test]
    fn nested_scroll_offsets_accumulate() {
        // scrollA(0,10) ⊃ scrollB(0,20) ⊃ leaf（均 overflow:scroll）
        // leaf world.apply(0,0) = (0, -30)（吃 A 的 -10 + B 的 -20，累积）
        let mut s = scene_with(vec![
            scroll_node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }),
            scroll_node(1, Some(0), Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 }),
            node(2, Some(1), Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }),
        ]);
        let a_id = root_id(&s);
        let b_id = child_id(&s, a_id, 0);
        let leaf_id = child_id(&s, b_id, 0);
        s.scroll.ensure(a_id).scroll_pos = (0.0, 10.0);
        s.scroll.ensure(b_id).scroll_pos = (0.0, 20.0);
        compute_world_transforms(&mut s);
        let (x, y) = s.world_transforms[leaf_id.index()].apply_point(0.0, 0.0);
        assert!((x - 0.0).abs() < 1e-3 && (y - (-30.0)).abs() < 1e-3,
            "嵌套累积：A(-10) + B(-20) = -30");
    }
}
