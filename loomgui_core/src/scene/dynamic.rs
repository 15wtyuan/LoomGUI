//! 动态树操作（T5+T6）：运行时删/建/改节点。
//!
//! T5 实现 `remove_node`（递归删子 + 联动清 anim/scroll/tween + slotmap remove）。
//! T6 加动态建树/改树 API：`kind_from_tag` / `apply_css` / `create_node` / `create_root`
//! / `append_child` / `insert_before` / `remove_child`（摘除不删）/ `set_text` / `set_src` / `set_style`。
//!
//! **设计要点**（spec §5.3 + §7 + §8）：
//! - 删节点联动清持久附属 map（anim/scroll remove + tween kill），防悬空 NodeId 残留
//!   写幽灵槽（HashMap 对任意 NodeId 都能插条目，须显式 remove）。
//! - 递归删子先 clone children 再递归（避免边迭代边改 slotmap 的借用冲突）。
//! - slotmap remove 后旧 NodeId 失效（gen++，Scene::get 返 None），槽位可复用。
//! - 动态建树复用 `mapping::apply_decl`（runtime 可用，不依赖 parse feature）做 CSS 声明应用，
//!   复用 dom.rs 围栏白名单语义做 tag→NodeKind（`kind_from_tag`，Task 7 提取复用）。
//! - create_node 填 base_style（源）+ style=base_style.clone()（派生），下帧 rematch 从 base 起算。

use crate::scene::node::{Node, NodeId, NodeKind, Rect, Scene};
use crate::style::mapping::apply_decl;
use crate::style::resolved::{OverflowMode, ResolvedStyle};
use crate::tween::TweenManager;

/// tag 字符串 → NodeKind（复用 dom.rs 围栏白名单语义，runtime 可用，不依赖 parse feature）。
/// 围栏白名单：div/l-container→Container, button→Button, img→Image, span→Text。
/// 未识别 tag → Err（动态建树 API 的 kind 入参由调用方负责，不像 parse 层有白名单兜底）。
pub fn kind_from_tag(tag: &str) -> Result<NodeKind, String> {
    match tag {
        "div" | "l-container" => Ok(NodeKind::Container),
        "button" => Ok(NodeKind::Button),
        "img" => Ok(NodeKind::Image { src: String::new() }),
        "span" => Ok(NodeKind::Text { content: String::new() }),
        other => Err(format!(
            "unknown kind tag: {}（围栏白名单：div/l-container/button/img/span）",
            other
        )),
    }
}

/// CSS 声明串（"width:100px;background:#f00"）→ 应用到 ResolvedStyle。
/// 极简分割（split(';') + split_once(':')），逐条调 `mapping::apply_decl`。
/// 不识别的声明静默忽略（apply_decl 返 false）；格式错（无冒号）的声明跳过。
/// runtime 可用，不依赖 parse feature（apply_decl 是 mapping.rs 默认编译的公共函数）。
pub fn apply_css(style: &mut ResolvedStyle, css: &str) {
    for decl in css.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        if let Some((prop, val)) = decl.split_once(':') {
            apply_decl(style, prop.trim(), val.trim());
        }
    }
}

/// 建节点：kind_from_tag + apply_css 填 base_style + slotmap insert + 回填 node.id。
/// base_style = apply_css 结果（源），style 初始 = base_style.clone()（派生，下帧 rematch 从 base 起算）。
/// clip_rect 按 overflow_x/y（非 Visible）派生 Some(占位)（值由 layout/render 填）。
/// anim/scroll 不预填（HashMap 懒初始化，ensure 时填）。返回新 NodeId。
pub fn create_node(scene: &mut Scene, kind: &str, css: &str) -> Result<NodeId, String> {
    let k = kind_from_tag(kind)?;
    let mut base_style = ResolvedStyle::default();
    apply_css(&mut base_style, css);
    let touchable = base_style.touchable;
    let clip = if base_style.overflow_x != OverflowMode::Visible
        || base_style.overflow_y != OverflowMode::Visible
    {
        Some(Rect::default())
    } else {
        None
    };
    let dirty_text = matches!(k, NodeKind::Text { .. });
    let node = Node {
        id: NodeId::INVALID, // 临时，insert 后回填
        parent: None,
        kind: k,
        style: base_style.clone(),
        base_style,
        taffy_id: None,
        layout_rect: Rect::default(),
        clip_rect: clip,
        children: Vec::new(),
        dirty_mesh: true,
        dirty_text,
        classes: Vec::new(),
        id_attr: None,
        touchable,
        hovered: false,
        active: false,
        disabled: false,
        draggable: false,
        tabindex: None,
        focused: false,
    };
    let key = scene.nodes.insert(node);
    let id = NodeId::from_key(key);
    scene.nodes.get_mut(key).unwrap().id = id; // 回填
    Ok(id)
}

/// 建根节点：create_node + roots.push(id)。
pub fn create_root(scene: &mut Scene, kind: &str, css: &str) -> Result<NodeId, String> {
    let id = create_node(scene, kind, css)?;
    scene.roots.push(id);
    Ok(id)
}

/// 挂子：parent.children 末尾追加 + child.parent = Some(parent)。
/// child 必须当前无父（先 remove_child 摘除旧父）。重复挂同一父子对幂等（已含则 no-op）。
pub fn append_child(scene: &mut Scene, parent: NodeId, child: NodeId) -> Result<(), String> {
    // 先做存在性 + 无父检查（不可变借），drop 后再可变借写。
    {
        let p = scene.get(parent).ok_or("parent not live")?;
        if p.children.contains(&child) {
            return Ok(()); // 幂等：已挂同一父子对
        }
        if scene.get(child).and_then(|c| c.parent).is_some() {
            return Err("child already has parent（先 remove_child 摘除旧父）".into());
        }
    }
    scene.get_mut(parent).unwrap().children.push(child);
    scene.get_mut(child).unwrap().parent = Some(parent);
    Ok(())
}

/// 插子：在 parent.children 中 ref_id 之前插入 child。ref_id=INVALID → 末尾追加（同 append_child）。
/// child 必须当前无父。ref_id 必须在 parent.children 中。
pub fn insert_before(
    scene: &mut Scene,
    parent: NodeId,
    child: NodeId,
    ref_id: NodeId,
) -> Result<(), String> {
    if !ref_id.is_valid() {
        return append_child(scene, parent, child);
    }
    if scene.get(child).and_then(|c| c.parent).is_some() {
        return Err("child already has parent（先 remove_child 摘除旧父）".into());
    }
    let p = scene.get_mut(parent).ok_or("parent not live")?;
    let pos = p
        .children
        .iter()
        .position(|&c| c == ref_id)
        .ok_or("ref_id not in parent.children")?;
    p.children.insert(pos, child);
    scene.get_mut(child).unwrap().parent = Some(parent);
    Ok(())
}

/// 摘子：从 parent.children 移除 child + child.parent = None。
/// 与 remove_node 不同——节点不删（slotmap 槽保留，NodeId 仍 live），可再挂到别处。
pub fn remove_child(scene: &mut Scene, parent: NodeId, child: NodeId) -> Result<(), String> {
    let p = scene.get_mut(parent).ok_or("parent not live")?;
    p.children.retain(|&c| c != child);
    if let Some(c) = scene.get_mut(child) {
        c.parent = None;
    }
    Ok(())
}

/// 改 Text 节点 content + 标 dirty_text。非 Text 节点 → Err。
pub fn set_text(scene: &mut Scene, node: NodeId, text: &str) -> Result<(), String> {
    let n = scene.get_mut(node).ok_or("node not live")?;
    match &mut n.kind {
        NodeKind::Text { content } => {
            *content = text.into();
        }
        _ => return Err("set_text 只对 Text 节点生效".into()),
    }
    n.dirty_text = true;
    Ok(())
}

/// 改 Image 节点 src + 标 dirty_mesh。非 Image 节点 → Err。
pub fn set_src(scene: &mut Scene, node: NodeId, src: &str) -> Result<(), String> {
    let n = scene.get_mut(node).ok_or("node not live")?;
    match &mut n.kind {
        NodeKind::Image { src: s } => {
            *s = src.into();
        }
        _ => return Err("set_src 只对 Image 节点生效".into()),
    }
    n.dirty_mesh = true;
    Ok(())
}

/// 改 base_style（apply_css）+ 标 dirty_mesh。
/// 下帧 rematch_pseudo_classes 从 base_style 起算重算 style（T5 确认 rematch 已从 base 起算）。
pub fn set_style(scene: &mut Scene, node: NodeId, css: &str) -> Result<(), String> {
    let n = scene.get_mut(node).ok_or("node not live")?;
    apply_css(&mut n.base_style, css);
    n.dirty_mesh = true;
    Ok(())
}

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

    // ---- T6 动态建树 API 单元测试（自由函数级，不依赖 Stage） ----

    fn empty_scene() -> Scene {
        Scene {
            roots: Vec::new(),
            nodes: slotmap::SlotMap::with_key(),
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(),
            anim: Default::default(),
            scroll: Default::default(),
            text_layouts: Vec::new(),
        }
    }

    #[test]
    fn kind_from_tag_maps_fence_whitelist() {
        assert!(matches!(kind_from_tag("div").unwrap(), NodeKind::Container));
        assert!(matches!(kind_from_tag("l-container").unwrap(), NodeKind::Container));
        assert!(matches!(kind_from_tag("button").unwrap(), NodeKind::Button));
        assert!(matches!(kind_from_tag("img").unwrap(), NodeKind::Image { .. }));
        assert!(matches!(kind_from_tag("span").unwrap(), NodeKind::Text { .. }));
    }

    #[test]
    fn kind_from_tag_unknown_returns_err() {
        assert!(kind_from_tag("ul").is_err());
        assert!(kind_from_tag("").is_err());
    }

    #[test]
    fn apply_css_sets_width_and_background() {
        let mut s = ResolvedStyle::default();
        apply_css(&mut s, "width:100px;height:50px;background-color:#ff0000");
        use taffy::style::Dimension;
        assert!(matches!(s.taffy_style.size.width, Dimension::Length(100.0)));
        assert!(matches!(s.taffy_style.size.height, Dimension::Length(50.0)));
        assert_eq!(s.background_color, Some([1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn apply_css_ignores_empty_and_malformed() {
        let mut s = ResolvedStyle::default();
        // 空串 / 纯空白 / 无冒号 / 空声明 → 不 panic，不误改
        apply_css(&mut s, "");
        apply_css(&mut s, "   ;  ; ");
        apply_css(&mut s, "noscolon");
        apply_css(&mut s, "width:200px");
        use taffy::style::Dimension;
        assert!(matches!(s.taffy_style.size.width, Dimension::Length(200.0)));
    }

    #[test]
    fn apply_css_unknown_decl_silently_ignored() {
        let mut s = ResolvedStyle::default();
        apply_css(&mut s, "unknown-prop:42px;width:100px");
        use taffy::style::Dimension;
        assert!(matches!(s.taffy_style.size.width, Dimension::Length(100.0)), "known 声明生效");
    }

    #[test]
    fn create_node_fills_base_style_and_id() {
        let mut scene = empty_scene();
        let id = create_node(&mut scene, "div", "width:100px;height:100px").unwrap();
        let n = scene.get(id).unwrap();
        assert_eq!(n.id, id, "id 回填");
        assert!(n.parent.is_none());
        use taffy::style::Dimension;
        assert!(matches!(n.base_style.taffy_style.size.width, Dimension::Length(100.0)));
        // style 初始 = base_style.clone()
        assert_eq!(n.style, n.base_style);
        assert!(n.dirty_mesh, "新建节点 dirty_mesh=true");
    }

    #[test]
    fn create_node_text_marks_dirty_text() {
        let mut scene = empty_scene();
        let id = create_node(&mut scene, "span", "").unwrap();
        let n = scene.get(id).unwrap();
        assert!(n.dirty_text, "Text 节点 dirty_text=true");
        assert!(matches!(n.kind, NodeKind::Text { .. }));
    }

    #[test]
    fn create_node_clip_rect_for_overflow_hidden() {
        let mut scene = empty_scene();
        let id = create_node(&mut scene, "div", "overflow:hidden").unwrap();
        assert!(scene.get(id).unwrap().clip_rect.is_some(), "overflow:hidden → clip slot");
        let id2 = create_node(&mut scene, "div", "").unwrap();
        assert!(scene.get(id2).unwrap().clip_rect.is_none(), "默认 Visible → 无 clip slot");
    }

    #[test]
    fn create_root_pushes_to_roots() {
        let mut scene = empty_scene();
        let r = create_root(&mut scene, "div", "").unwrap();
        assert_eq!(scene.roots, vec![r]);
    }

    #[test]
    fn append_child_links_parent_and_child() {
        let mut scene = empty_scene();
        let root = create_root(&mut scene, "div", "").unwrap();
        let child = create_node(&mut scene, "div", "").unwrap();
        append_child(&mut scene, root, child).unwrap();
        assert_eq!(scene.get(root).unwrap().children, vec![child]);
        assert_eq!(scene.get(child).unwrap().parent, Some(root));
    }

    #[test]
    fn append_child_idempotent_same_pair() {
        let mut scene = empty_scene();
        let root = create_root(&mut scene, "div", "").unwrap();
        let child = create_node(&mut scene, "div", "").unwrap();
        append_child(&mut scene, root, child).unwrap();
        // 二次挂同一对 → 幂等 no-op（不报错，children 不重复）
        append_child(&mut scene, root, child).unwrap();
        assert_eq!(scene.get(root).unwrap().children.len(), 1);
    }

    #[test]
    fn append_child_rejects_child_with_existing_parent() {
        let mut scene = empty_scene();
        let a = create_root(&mut scene, "div", "").unwrap();
        let b = create_node(&mut scene, "div", "").unwrap();
        let c = create_node(&mut scene, "div", "").unwrap();
        append_child(&mut scene, a, c).unwrap();
        // c 已有父 a → 挂到 b 应报错
        let err = append_child(&mut scene, b, c);
        assert!(err.is_err(), "child 已有父 → Err");
    }

    #[test]
    fn insert_before_inserts_in_middle() {
        let mut scene = empty_scene();
        let root = create_root(&mut scene, "div", "").unwrap();
        let a = create_node(&mut scene, "div", "").unwrap();
        let b = create_node(&mut scene, "div", "").unwrap();
        let c = create_node(&mut scene, "div", "").unwrap();
        append_child(&mut scene, root, a).unwrap();
        append_child(&mut scene, root, b).unwrap();
        // 在 a 之前插 c → [c, a, b]
        insert_before(&mut scene, root, c, a).unwrap();
        assert_eq!(scene.get(root).unwrap().children, vec![c, a, b]);
        assert_eq!(scene.get(c).unwrap().parent, Some(root));
    }

    #[test]
    fn insert_before_invalid_ref_appends() {
        let mut scene = empty_scene();
        let root = create_root(&mut scene, "div", "").unwrap();
        let a = create_node(&mut scene, "div", "").unwrap();
        let b = create_node(&mut scene, "div", "").unwrap();
        append_child(&mut scene, root, a).unwrap();
        // ref=INVALID → 末尾追加
        insert_before(&mut scene, root, b, NodeId::INVALID).unwrap();
        assert_eq!(scene.get(root).unwrap().children, vec![a, b]);
    }

    #[test]
    fn insert_before_missing_ref_returns_err() {
        let mut scene = empty_scene();
        let root = create_root(&mut scene, "div", "").unwrap();
        let a = create_node(&mut scene, "div", "").unwrap();
        append_child(&mut scene, root, a).unwrap();
        // 造一个 valid 但不在 root.children 的 NodeId 作 ref
        let orphan = create_node(&mut scene, "div", "").unwrap();
        let new_child = create_node(&mut scene, "div", "").unwrap();
        let err = insert_before(&mut scene, root, new_child, orphan);
        assert!(err.is_err(), "ref 不在 parent.children → Err");
    }

    #[test]
    fn remove_child_detaches_but_keeps_node() {
        let mut scene = empty_scene();
        let root = create_root(&mut scene, "div", "").unwrap();
        let child = create_node(&mut scene, "div", "").unwrap();
        append_child(&mut scene, root, child).unwrap();
        remove_child(&mut scene, root, child).unwrap();
        assert!(scene.get(root).unwrap().children.is_empty());
        assert!(scene.get(child).unwrap().parent.is_none(), "child 变孤立");
        assert!(scene.get(child).is_some(), "child 仍存活（未删 slotmap 槽）");
    }

    #[test]
    fn set_text_changes_content_and_marks_dirty() {
        let mut scene = empty_scene();
        let t = create_node(&mut scene, "span", "").unwrap();
        // create_node 时 dirty_text=true（Text 节点），先清掉验 set_text 重标
        scene.get_mut(t).unwrap().dirty_text = false;
        set_text(&mut scene, t, "hello").unwrap();
        assert!(scene.get(t).unwrap().dirty_text);
        match &scene.get(t).unwrap().kind {
            NodeKind::Text { content } => assert_eq!(content, "hello"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn set_text_rejects_non_text() {
        let mut scene = empty_scene();
        let d = create_node(&mut scene, "div", "").unwrap();
        assert!(set_text(&mut scene, d, "x").is_err());
    }

    #[test]
    fn set_src_changes_src_and_marks_dirty_mesh() {
        let mut scene = empty_scene();
        let img = create_node(&mut scene, "img", "").unwrap();
        scene.get_mut(img).unwrap().dirty_mesh = false;
        set_src(&mut scene, img, "icon.png").unwrap();
        assert!(scene.get(img).unwrap().dirty_mesh);
        match &scene.get(img).unwrap().kind {
            NodeKind::Image { src } => assert_eq!(src, "icon.png"),
            _ => panic!("expected Image"),
        }
    }

    #[test]
    fn set_src_rejects_non_image() {
        let mut scene = empty_scene();
        let d = create_node(&mut scene, "div", "").unwrap();
        assert!(set_src(&mut scene, d, "x").is_err());
    }

    #[test]
    fn set_style_changes_base_style_marks_dirty() {
        let mut scene = empty_scene();
        let n = create_node(&mut scene, "div", "").unwrap();
        scene.get_mut(n).unwrap().dirty_mesh = false;
        set_style(&mut scene, n, "background-color:#ff0000").unwrap();
        let bg = scene.get(n).unwrap().base_style.background_color;
        assert_eq!(bg, Some([1.0, 0.0, 0.0, 1.0]));
        assert!(scene.get(n).unwrap().dirty_mesh, "set_style 标 dirty_mesh");
    }

    #[test]
    fn create_node_id_is_live_via_get() {
        // slotmap insert 返回的 NodeId 经 from_key 转换，Scene::get 能查到（to_key roundtrip）
        let mut scene = empty_scene();
        let id = create_node(&mut scene, "div", "").unwrap();
        assert!(scene.get(id).is_some(), "create_node 返回的 NodeId live");
        assert_ne!(id, NodeId::INVALID);
    }

    #[test]
    fn append_child_builds_multi_level_tree() {
        let mut scene = empty_scene();
        let root = create_root(&mut scene, "div", "").unwrap();
        let c1 = create_node(&mut scene, "div", "").unwrap();
        let c2 = create_node(&mut scene, "div", "").unwrap();
        append_child(&mut scene, root, c1).unwrap();
        append_child(&mut scene, c1, c2).unwrap();
        assert_eq!(scene.get(root).unwrap().children, vec![c1]);
        assert_eq!(scene.get(c1).unwrap().children, vec![c2]);
        assert_eq!(scene.get(c2).unwrap().parent, Some(c1));
    }
}
