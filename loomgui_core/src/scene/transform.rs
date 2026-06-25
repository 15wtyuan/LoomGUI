//! v1d.3：每帧 DFS 算每节点累计世界矩阵（spec §3.2）。
//! pivot = box center（w/2,h/2）；local = T(rel) ∘ T(pivot) ∘ transform ∘ T(-pivot)；
//! world = parent.world ∘ local。

use crate::scene::node::{AnimTable, NodeId, Scene};
use crate::transform::{self, Affine2};

pub fn compute_world_transforms(scene: &mut Scene) {
    let n = scene.nodes.len();
    let mut worlds: Vec<Affine2> = vec![transform::IDENTITY; n];
    let roots = scene.roots.clone();
    for root in roots {
        rec(scene, &scene.anim, root, transform::IDENTITY, &mut worlds);
    }
    scene.world_transforms = worlds;
}

fn rec(scene: &Scene, anim: &AnimTable, id: NodeId, parent_world: Affine2, worlds: &mut [Affine2]) {
    let node = &scene.nodes[id.0];
    let lr = node.layout_rect;
    let pivot = (lr.w / 2.0, lr.h / 2.0);
    let rel = match node.parent {
        Some(p) => (lr.x - scene.nodes[p.0].layout_rect.x, lr.y - scene.nodes[p.0].layout_rect.y),
        None => (lr.x, lr.y),
    };
    // v1d.4：transform 矩阵源 = anim.transform override（replace-override）unwrap css matrix。
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
    let world = transform::mul(&parent_world, &local);
    worlds[id.0] = world;
    let kids = node.children.clone();
    for c in kids {
        rec(scene, anim, c, world, worlds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::{Node, Rect, Scene};
    use crate::style::resolved::LocalTransform;
    use crate::transform::Affine2Ext;

    fn node(id: usize, parent: Option<usize>, rect: Rect) -> Node {
        let mut n = Node::default();
        n.id = NodeId(id);
        n.parent = parent.map(NodeId);
        n.layout_rect = rect;
        n
    }

    fn scene_with(nodes: Vec<Node>) -> Scene {
        let roots = nodes.iter().find(|n| n.parent.is_none()).map(|n| n.id).into_iter().collect();
        Scene { roots, nodes, dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default() }
    }

    #[test]
    fn identity_nodes_world_is_translation_to_layout() {
        // parent identity at (10,20,100,100)；child identity at (15,25,...)（rel 5,5）
        let mut s = scene_with(vec![
            node(0, None, Rect { x: 10.0, y: 20.0, w: 100.0, h: 100.0 }),
            { let c = node(1, Some(0), Rect { x: 15.0, y: 25.0, w: 10.0, h: 10.0 }); c },
        ]);
        s.nodes[0].children = vec![NodeId(1)];
        compute_world_transforms(&mut s);
        // child world = T(10,20) ∘ T(5,5) = T(15,25)；纯平移，apply(0,0)=(15,25)
        let (x, y) = s.world_transforms[1].apply_point(0.0, 0.0);
        assert!((x - 15.0).abs() < 1e-4 && (y - 25.0).abs() < 1e-4);
        assert!(s.world_transforms[1].is_pure_translation(), "identity 子树 world 纯平移");
    }

    #[test]
    fn child_inherits_parent_rotation() {
        // parent rotate(90°) at (0,0,100,100)；child identity at (0,0,10,10)
        let mut s = scene_with(vec![
            node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }),
            node(1, Some(0), Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }),
        ]);
        s.nodes[0].children = vec![NodeId(1)];
        s.nodes[0].style.transform = LocalTransform { matrix: transform::from_rotate(std::f32::consts::FRAC_PI_2) };
        compute_world_transforms(&mut s);
        // child world == parent world（child identity local=identity）
        assert_eq!(s.world_transforms[1], s.world_transforms[0], "identity 子继承父 world");
        assert!(!s.world_transforms[0].is_pure_translation(), "父旋转 → world 非纯平移");
    }

    #[test]
    fn skew_composite_propagates_to_child_world() {
        // parent scale(2,1) at (0,0,100,100)；child rotate(45°) → child world 含剪切
        let mut s = scene_with(vec![
            node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }),
            node(1, Some(0), Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }),
        ]);
        s.nodes[0].children = vec![NodeId(1)];
        s.nodes[0].style.transform = LocalTransform { matrix: transform::from_scale(2.0, 1.0) };
        s.nodes[1].style.transform = LocalTransform { matrix: transform::from_rotate(std::f32::consts::FRAC_PI_4) };
        compute_world_transforms(&mut s);
        assert!(!s.world_transforms[1].is_pure_translation(), "父非均匀缩放+子旋转 → world 剪切（非纯平移）");
    }

    #[test]
    fn translate_stacks_on_layout_rel() {
        // parent identity at (0,0,100,100)；child translate(5,0) layout at (10,0)（rel 10,0）
        // child world.apply(0,0) = (15,0)（rel 10 + translate 5）
        let mut s = scene_with(vec![
            node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }),
            node(1, Some(0), Rect { x: 10.0, y: 0.0, w: 10.0, h: 10.0 }),
        ]);
        s.nodes[0].children = vec![NodeId(1)];
        s.nodes[1].style.transform = LocalTransform { matrix: transform::from_translate(5.0, 0.0) };
        compute_world_transforms(&mut s);
        let (x, y) = s.world_transforms[1].apply_point(0.0, 0.0);
        assert!((x - 15.0).abs() < 1e-4 && y.abs() < 1e-4, "translate 叠 layout rel 不双计：rel(10)+t(5)=15");
    }

    #[test]
    fn anim_transform_override_replaces_css_matrix() {
        // node CSS transform=identity；anim.transform=scale(2,2) → world 非纯平移（吃 override）
        let mut s = scene_with(vec![ node(0, None, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }) ]);
        s.anim.ensure(1);
        s.anim.0[0].transform = Some(transform::from_scale(2.0, 2.0));
        compute_world_transforms(&mut s);
        assert!(!s.world_transforms[0].is_pure_translation(), "anim.transform override 生效（scale）");
    }

    #[test]
    fn no_anim_falls_back_to_css_matrix_zero_regression() {
        // anim 全 None → world == CSS（identity）纯平移到 layout 原点
        let mut s = scene_with(vec![ node(0, None, Rect { x: 5.0, y: 7.0, w: 10.0, h: 10.0 }) ]);
        // 不设 anim（全 None）
        compute_world_transforms(&mut s);
        let (x, y) = s.world_transforms[0].apply_point(0.0, 0.0);
        assert!((x - 5.0).abs() < 1e-4 && (y - 7.0).abs() < 1e-4, "无 anim → CSS identity 纯平移");
        assert!(s.world_transforms[0].is_pure_translation());
    }
}
