//! 动态树操作（T5+）：运行时删/建节点。
//!
//! T5 实现 `remove_node`（递归删子 + 联动清 anim/scroll/tween + slotmap remove）。
//! T6 将加 `create_node` / `append_child` / `insert_child` / `remove_child`（摘除不删）
//! / `set_text` / `set_src` / `set_style` 等动态建树/改树 API。
//!
//! **设计要点**（spec §5.3 + §8）：
//! - 删节点联动清持久附属 map（anim/scroll remove + tween kill），防悬空 NodeId 残留
//!   写幽灵槽（HashMap 对任意 NodeId 都能插条目，须显式 remove）。
//! - 递归删子先 clone children 再递归（避免边迭代边改 slotmap 的借用冲突）。
//! - slotmap remove 后旧 NodeId 失效（gen++，Scene::get 返 None），槽位可复用。

use crate::scene::node::{NodeId, Scene};
use crate::tween::TweenManager;

/// 删节点：递归删子 → 从父/roots 摘除 → 联动清 anim/scroll/tween → slotmap remove。
///
/// 旧 NodeId 此后失效（slotmap gen++，Scene::get 返 None）。子树递归删。
/// anim/scroll/tween 联动清（HashMap remove / tween kill），防悬空残留。
/// 槽位可复用（slotmap remove 释放槽，下次 insert 复用 + gen++）。
///
/// 调用方须保证 `id` 为 live NodeId（已删节点 no-op：scene.get 返 None 直接返回）。
pub fn remove_node(scene: &mut Scene, tweens: &mut TweenManager, id: NodeId) {
    // 0. 已删/无效节点 → no-op（防重复删或悬空 id 调用 panic）。
    //    先取 children + parent（持有不可变借），drop 后再递归/可变借。
    let (children, parent_id) = match scene.get(id) {
        Some(n) => (n.children.clone(), n.parent),
        None => return,
    };
    // 1. 递归删子（先 clone 了 children，避免边迭代边改 slotmap）。
    for c in children {
        remove_node(scene, tweens, c);
    }
    // 2. 从父摘除（或 roots）
    match parent_id {
        Some(pid) => {
            if let Some(p) = scene.get_mut(pid) {
                p.children.retain(|&c| c != id);
            }
        }
        None => scene.roots.retain(|&r| r != id),
    }
    // 3. 联动清持久附属 map（anim/scroll HashMap remove + tween kill）。
    scene.anim.clear_node(id);
    scene.scroll.remove(id);
    tweens.kill_node(id);
    // 4. slotmap remove（gen++，旧 NodeId 失效，槽位可复用）。
    //    经 key_for(NodeId) 桥接到 DefaultKey（T2）。
    scene.nodes.remove(scene.key_for(id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::NodeKind;
    use crate::style::resolved::ResolvedStyle;
    use crate::tween::{Ease, TweenProp};

    /// 建 3 层树：root → child → grandchild。用 Scene::build（不依赖 T6 动态建树 API）。
    fn build_3level() -> (Scene, NodeId, NodeId, NodeId) {
        let entries: Vec<(
            Option<usize>,
            NodeKind,
            ResolvedStyle,
            Vec<String>,
            Option<String>,
            bool,
            Option<i32>,
        )> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(1), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let scene = Scene::build(&entries);
        let root = scene.roots[0];
        let child = scene.get(root).unwrap().children[0];
        let grand = scene.get(child).unwrap().children[0];
        (scene, root, child, grand)
    }

    #[test]
    fn remove_node_clears_anim_scroll_and_kills_tween() {
        let (mut scene, root, child, _grand) = build_3level();
        let mut tweens = TweenManager::new();
        // 给 child 灌 anim/scroll/tween
        scene.anim.ensure(child).opacity = Some(0.5);
        scene.scroll.ensure(child);
        tweens.tween(child, TweenProp::Opacity,
            [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
            Ease::Linear, 0.0, 1.0, 0);
        // 删 child
        remove_node(&mut scene, &mut tweens, child);
        // 联动清
        assert!(scene.anim.get(child).is_none(), "anim 清");
        assert!(scene.scroll.get(child).is_none(), "scroll 清");
        assert!(tweens.tweens.iter().all(|t| t.node != child || t.killed), "tween killed");
        assert!(scene.get(child).is_none(), "slotmap removed（旧 NodeId 失效）");
        // root 仍在，且 root.children 不再含 child
        assert!(scene.get(root).is_some(), "root 未删");
        assert!(!scene.get(root).unwrap().children.contains(&child), "child 从父摘除");
    }

    #[test]
    fn remove_node_recurses_children() {
        let (mut scene, root, child, grand) = build_3level();
        let mut tweens = TweenManager::new();
        // 给 grand 灌 anim
        scene.anim.ensure(grand).opacity = Some(0.5);
        // 删 root → 递归删 child + grand
        remove_node(&mut scene, &mut tweens, root);
        assert!(scene.get(root).is_none(), "root 删");
        assert!(scene.get(child).is_none(), "子递归删");
        assert!(scene.get(grand).is_none(), "孙递归删");
        assert!(scene.anim.get(grand).is_none(), "孙 anim 联动清");
        assert!(scene.roots.is_empty(), "roots 摘除");
    }

    #[test]
    fn remove_node_from_middle_clears_subtree_and_keeps_siblings() {
        // root → [a, b, c]；删 b → a/c 保留，b 子树（b → bchild）递归删。
        let entries: Vec<(
            Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>,
        )> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(2), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        let mut tweens = TweenManager::new();
        let root = scene.roots[0];
        let kids = scene.get(root).unwrap().children.clone();
        let (a, b, c) = (kids[0], kids[1], kids[2]);
        let bchild = scene.get(b).unwrap().children[0];
        scene.anim.ensure(bchild).opacity = Some(0.5);
        // 删 b
        remove_node(&mut scene, &mut tweens, b);
        assert!(scene.get(a).is_some(), "兄弟 a 保留");
        assert!(scene.get(c).is_some(), "兄弟 c 保留");
        assert!(scene.get(b).is_none(), "b 删");
        assert!(scene.get(bchild).is_none(), "bchild 递归删");
        assert!(scene.anim.get(bchild).is_none(), "bchild anim 清");
        // root.children 不再含 b，但含 a/c
        let new_kids = scene.get(root).unwrap().children.clone();
        assert!(!new_kids.contains(&b), "b 从父摘除");
        assert!(new_kids.contains(&a) && new_kids.contains(&c), "a/c 保留在父 children");
        assert_eq!(new_kids.len(), 2, "父 children 从 3 → 2");
    }

    #[test]
    fn remove_node_already_removed_is_noop() {
        let (mut scene, root, child, _grand) = build_3level();
        let mut tweens = TweenManager::new();
        // 删 child 两次：第二次 no-op（不 panic）
        remove_node(&mut scene, &mut tweens, child);
        remove_node(&mut scene, &mut tweens, child);
        assert!(scene.get(child).is_none());
        assert!(scene.get(root).is_some(), "root 仍在");
    }

    #[test]
    fn remove_node_slot_reuse_invalidates_old_nodeid() {
        // 删后槽位可复用：旧 NodeId 失效（gen++），新 insert 复用槽位但 NodeId 不同。
        let (mut scene, root, child, _grand) = build_3level();
        let mut tweens = TweenManager::new();
        let child_id_old = child;
        remove_node(&mut scene, &mut tweens, child);
        assert!(scene.get(child_id_old).is_none(), "旧 NodeId 失效（gen++）");
        // 新 insert（复用槽位）
        let new_key = scene.nodes.insert(crate::scene::node::Node::default());
        let new_id = crate::scene::node::NodeId::from_key(new_key);
        // 旧 child_id 与新 new_id 不同（gen 不同），旧 id 仍 None
        assert!(scene.get(child_id_old).is_none(), "旧 NodeId 仍失效");
        assert!(scene.get(new_id).is_some(), "新 NodeId live");
        // root 仍在
        assert!(scene.get(root).is_some());
    }
}
