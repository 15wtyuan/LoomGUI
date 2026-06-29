//! 运行时伪类重匹配的动态规则表。
//!
//! 本模块填实 `match_element_with_state`（完整后代链匹配 + 伪类状态门）+
//! `rematch_pseudo_classes`（全量节点重 cascade，写 Node.style + 标 layout dirty）。
//!
//! **常驻（不 gate）：**本模块的选择器数据模型（`ParsedSelector`/`Compound`/`Combinator`/
//! `Specificity`）+ `Declaration`（CSS 声明）+ `compound_matches_node`（运行时 compound 匹配）+
//! 动态规则匹配全不依赖 parse feature——bincode 序列化进 `.pkg.bin` 的就是这些结构
//! （runtime 不重新 parse，直接用反序列化结构）。`parse::selector`/`parse::css`
//! 只保留解析器函数（string → 这些结构），仍 `#[cfg(feature="parse")]`，本模块 `pub use` 重导出
//! 数据类型以维持路径兼容（`loomgui_core::parse::selector::ParsedSelector` 仍可达）。

use serde::{Deserialize, Serialize};

// ── 选择器数据模型（常驻；parse feature off 时仍可用于 bincode 反序列化 + rematch）──

/// CSS 声明（prop + value）。序列化进 .pkg.bin DynamicRuleSection。
/// 与 `parse::css::Declaration` 同型——parse feature 下 `parse::css` 重导出本类型保持路径兼容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Declaration {
    pub prop: String,
    pub value: String,
}

/// 选择器组合子：标签/类/id/后代/子代 + 伪类状态门（hover/active/disabled）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSelector {
    pub raw: String,
    pub compound: Vec<Compound>, // 复合选择器链（后代/子代分隔）
    pub specificity: Specificity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compound {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub id: Option<String>,
    pub combinator: Combinator, // 本 compound 与前一个的关系
    pub pseudo_hover: bool,
    pub pseudo_active: bool,
    pub pseudo_disabled: bool,
    pub pseudo_focus: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Combinator {
    Descendant,
    Child,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Specificity(pub u32, pub u32, pub u32); // (id 数, class 数, tag 数)

// ── 动态规则表（DynamicRule 持有 ParsedSelector + Declarations，均 bincode 可序列化）──

use crate::scene::node::{NodeId, Scene};
use crate::style::mapping::apply_decl;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DynamicRuleTable {
    pub rules: Vec<DynamicRule>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DynamicRule {
    pub selector: ParsedSelector,
    pub declarations: Vec<Declaration>,
}

use crate::scene::node::{Node, NodeKind};

/// 运行时版 compound 匹配（消费 Node 而非 ElementData，运行时无 ElementTree）。
/// 匹配 tag/classes/id（不含伪类状态——状态由 match_element_with_state 门控）。
/// id 属性存 Node.id_attr（`id="..."`）；Node.id 是 NodeId 索引身份，二者不同。
///
/// **常驻：**runtime rematch 用，不依赖 parse feature。
pub fn compound_matches_node(c: &Compound, node: &Node) -> bool {
    if let Some(t) = &c.tag {
        let kind_tag = match &node.kind {
            NodeKind::Container => "div",
            NodeKind::Button => "button",
            NodeKind::Image { .. } => "img",
            NodeKind::Text { .. } => "span",
        };
        if kind_tag != t.as_str() {
            return false;
        }
    }
    if let Some(id) = &c.id {
        if node.id_attr.as_deref() != Some(id.as_str()) {
            return false;
        }
    }
    for cls in &c.classes {
        if !node.classes.iter().any(|nc| nc == cls) {
            return false;
        }
    }
    true
}

/// 判定 compound 是否匹配 node + 伪类状态门。
/// `pseudo_hover → node.hovered`；`pseudo_active → node.active`；
/// `pseudo_disabled → node.disabled`；`pseudo_focus → node.focused`。
/// 先检状态门，再调 compound_matches_node（tag/classes/id_attr 匹配）。
fn compound_matches_with_state(c: &Compound, node_id: NodeId, scene: &Scene) -> bool {
    let node = &scene.nodes[node_id.0];
    if c.pseudo_hover && !node.hovered {
        return false;
    }
    if c.pseudo_active && !node.active {
        return false;
    }
    if c.pseudo_disabled && !node.disabled {
        return false;
    }
    if c.pseudo_focus && !node.focused {
        return false;
    }
    compound_matches_node(c, node)
}

/// 完整后代链匹配（从右往左，复用 selector.rs `matches` 算法，消费 Node/Scene）。
/// 最后一个 compound 必须命中目标 node 本身（含状态门）；前面按 combinator 沿
/// parent 链找（Child=直接父，Descendant=任一祖先，带回溯）。
pub fn match_element_with_state(sel: &ParsedSelector, node_id: NodeId, scene: &Scene) -> bool {
    let comps = &sel.compound;
    if comps.is_empty() {
        return false;
    }
    let last = &comps[comps.len() - 1];
    if !compound_matches_with_state(last, node_id, scene) {
        return false;
    }
    if comps.len() == 1 {
        return true;
    }
    match_chain_with_state(comps, comps.len() - 1, node_id, scene)
}

/// 递归匹配 comps[0..end_idx] 在 start_node 的祖先链上（同 selector.rs
/// `match_compound_chain`）。`start_node` 是已匹配 comps[end_idx] 的节点，
/// 为 comps[end_idx - 1] 找祖先（Child：直接父；Descendant：任一祖先+回溯）。
fn match_chain_with_state(
    comps: &[Compound],
    end_idx: usize,
    start_node: NodeId,
    scene: &Scene,
) -> bool {
    if end_idx == 0 {
        return true;
    }
    let target_comp = &comps[end_idx - 1];
    let combinator = comps[end_idx].combinator;
    match combinator {
        Combinator::Child => match scene.nodes[start_node.0].parent {
            Some(parent) => {
                compound_matches_with_state(target_comp, parent, scene)
                    && match_chain_with_state(comps, end_idx - 1, parent, scene)
            }
            None => false,
        },
        Combinator::Descendant => {
            let mut cur = scene.nodes[start_node.0].parent;
            while let Some(ancestor) = cur {
                if compound_matches_with_state(target_comp, ancestor, scene) {
                    if match_chain_with_state(comps, end_idx - 1, ancestor, scene) {
                        return true;
                    }
                    // 此祖先匹配但更左链匹配不上 → 继续往上找
                }
                cur = scene.nodes[ancestor.0].parent;
            }
            false
        }
    }
}

/// 全量节点重匹配（仅动态规则子集）。每节点从 base_style 重起，
/// 收集命中的动态规则（match_element_with_state），按 specificity 升序排
/// （高 specificity 后 apply 胜出）→ apply_decl 叠加 → 写 Node.style。
/// 返回是否有任何节点 layout 字段变（taffy_style + order）。solve 每帧全量，
/// 返回值仅供观测/测试，不驱动 solve。
pub fn rematch_pseudo_classes(scene: &mut Scene) -> bool {
    let mut any_layout_dirty = false;
    // 预提取 specificity 元组（避免每节点重解引用）
    let rules_with_spec: Vec<(u32, u32, u32, &DynamicRule)> = scene
        .dynamic_rules
        .rules
        .iter()
        .map(|r| {
            (
                r.selector.specificity.0,
                r.selector.specificity.1,
                r.selector.specificity.2,
                r,
            )
        })
        .collect();
    for i in 0..scene.nodes.len() {
        let node_id = NodeId(i);
        // 从 base_style 重起
        let mut new_style = scene.nodes[i].base_style.clone();
        // 收集命中规则
        let mut matched: Vec<(u32, u32, u32, &DynamicRule)> = Vec::new();
        for r in &rules_with_spec {
            if match_element_with_state(&r.3.selector, node_id, scene) {
                matched.push(*r);
            }
        }
        // specificity 升序（高 specificity 后 apply 胜出）；同级保持原序（stable sort）
        matched.sort_by_key(|r| (r.0, r.1, r.2));
        for (_, _, _, r) in &matched {
            for decl in &r.declarations {
                apply_decl(&mut new_style, &decl.prop, &decl.value);
            }
        }
        // 对比 layout 字段（taffy_style + order）——taffy 0.5 Style 已 derive PartialEq
        let old = &scene.nodes[i].style;
        let layout_changed =
            new_style.taffy_style != old.taffy_style || new_style.order != old.order;
        scene.nodes[i].style = new_style;
        if layout_changed {
            any_layout_dirty = true;
        }
    }
    any_layout_dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::css::Declaration;
    use crate::parse::selector::parse_selector;
    use crate::scene::node::{Node, NodeId, NodeKind, Rect, Scene};

    /// 构造 root + button(.btn) scene，button 在 (0,0,100,100)。
    fn btn_scene() -> Scene {
        let mut root = Node::default();
        root.id = NodeId(0);
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut btn = Node::default();
        btn.id = NodeId(1);
        btn.parent = Some(NodeId(0));
        btn.kind = NodeKind::Button;
        btn.classes = vec!["btn".to_string()];
        btn.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        root.children = vec![NodeId(1)];
        Scene {
            roots: vec![NodeId(0)],
            nodes: vec![root, btn],
            dynamic_rules: DynamicRuleTable::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        }
    }

    fn rule(sel: &str, prop: &str, val: &str) -> DynamicRule {
        DynamicRule {
            selector: parse_selector(sel).unwrap(),
            declarations: vec![Declaration {
                prop: prop.to_string(),
                value: val.to_string(),
            }],
        }
    }

    #[test]
    fn hover_pseudo_changes_background_color() {
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn:hover", "background-color", "#0000ff"));
        s.nodes[1].hovered = true; // 模拟命中 diff 后状态
        let changed = rematch_pseudo_classes(&mut s);
        // background_color 是视觉字段，不触发 layout dirty
        assert_eq!(
            s.nodes[1].style.background_color,
            Some([0.0, 0.0, 1.0, 1.0]),
            "hover → 蓝"
        );
        assert!(!changed, "仅视觉变 → layout 不 dirty");
    }

    #[test]
    fn active_pseudo_on_down() {
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn:active", "background-color", "#ff0000"));
        s.nodes[1].active = true;
        rematch_pseudo_classes(&mut s);
        assert_eq!(
            s.nodes[1].style.background_color,
            Some([1.0, 0.0, 0.0, 1.0]),
            "active → 红"
        );
    }

    #[test]
    fn disabled_pseudo_via_disabled_flag() {
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn:disabled", "opacity", "0.5"));
        s.nodes[1].disabled = true;
        rematch_pseudo_classes(&mut s);
        assert_eq!(s.nodes[1].style.opacity, 0.5, "disabled → opacity 0.5");
    }

    #[test]
    fn rematch_layout_dirty_when_size_changes() {
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn:hover", "width", "200px"));
        s.nodes[1].hovered = true;
        let changed = rematch_pseudo_classes(&mut s);
        assert!(changed, "width 变 → layout dirty");
        // 验 style.taffy_style.size.width 被改
        use taffy::style::Dimension;
        assert!(matches!(
            s.nodes[1].style.taffy_style.size.width,
            Dimension::Length(200.0)
        ));
    }

    #[test]
    fn rematch_no_dirty_when_only_visual_changes() {
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn:hover", "color", "#ff0000"));
        s.nodes[1].hovered = true;
        let changed = rematch_pseudo_classes(&mut s);
        assert!(!changed, "color 是视觉字段 → 不 layout dirty");
        assert_eq!(s.nodes[1].style.color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn descendant_pseudo_rule_matched() {
        // .parent:hover .child —— hover parent → child style 变（跨节点伪类联动）
        let mut root = Node::default();
        root.id = NodeId(0);
        root.classes = vec!["parent".to_string()];
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut child = Node::default();
        child.id = NodeId(1);
        child.parent = Some(NodeId(0));
        child.classes = vec!["child".to_string()];
        child.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 50.0,
        };
        root.children = vec![NodeId(1)];
        let mut s = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![root, child],
            dynamic_rules: DynamicRuleTable::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        s.dynamic_rules
            .rules
            .push(rule(".parent:hover .child", "color", "#0000ff"));
        s.nodes[0].hovered = true; // parent hovered
        rematch_pseudo_classes(&mut s);
        assert_eq!(
            s.nodes[1].style.color,
            [0.0, 0.0, 1.0, 1.0],
            "parent:hover → child 变蓝"
        );
    }

    #[test]
    fn no_pseudo_rule_not_in_dynamic_rules() {
        // 纯静态规则不进 dynamic_rules（打包器分流）——rematch 不区分有无伪类，
        // 只看状态门。若纯静态规则混进 dynamic，hovered=true 时仍匹配（无伪类规则恒匹配）。
        // 打包器保证无伪类规则不进 dynamic_rules。此测断言 color 变红。
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn", "color", "#ff0000"));
        s.nodes[1].hovered = true;
        rematch_pseudo_classes(&mut s);
        assert_eq!(s.nodes[1].style.color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn rematch_resets_to_base_when_no_rule_matches() {
        // hover 后变蓝 → hover 取消 → rematch 应回 base_style
        let mut s = btn_scene();
        s.nodes[1].base_style.background_color = None; // base 无 bg
        s.dynamic_rules
            .rules
            .push(rule(".btn:hover", "background-color", "#0000ff"));
        s.nodes[1].hovered = true;
        rematch_pseudo_classes(&mut s);
        assert_eq!(
            s.nodes[1].style.background_color,
            Some([0.0, 0.0, 1.0, 1.0])
        );
        s.nodes[1].hovered = false; // 取消 hover
        rematch_pseudo_classes(&mut s);
        assert_eq!(
            s.nodes[1].style.background_color,
            None,
            "取消 hover → 回 base"
        );
    }

    #[test]
    fn focus_pseudo_matches_focused_node() {
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn:focus", "background-color", "#0000ff"));
        s.nodes[1].focused = true;
        let changed = rematch_pseudo_classes(&mut s);
        assert_eq!(
            s.nodes[1].style.background_color,
            Some([0.0, 0.0, 1.0, 1.0]),
            "focused → :focus 匹配 → 蓝"
        );
        assert!(!changed, "background_color 视觉字段 → layout 不 dirty");
    }

    #[test]
    fn focus_pseudo_no_match_unfocused() {
        let mut s = btn_scene();
        s.nodes[1].base_style.background_color = None;
        s.dynamic_rules
            .rules
            .push(rule(".btn:focus", "background-color", "#0000ff"));
        s.nodes[1].focused = false;
        rematch_pseudo_classes(&mut s);
        assert_eq!(
            s.nodes[1].style.background_color,
            None,
            "unfocused → :focus 不匹配 → 回 base"
        );
    }

    #[test]
    fn focus_pseudo_in_descendant_chain() {
        // .parent:focus .child —— parent 聚焦 → child style 变
        let mut root = Node::default();
        root.id = NodeId(0);
        root.classes = vec!["parent".to_string()];
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut child = Node::default();
        child.id = NodeId(1);
        child.parent = Some(NodeId(0));
        child.classes = vec!["child".to_string()];
        child.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 50.0,
        };
        root.children = vec![NodeId(1)];
        let mut s = Scene {
            roots: vec![NodeId(0)],
            nodes: vec![root, child],
            dynamic_rules: DynamicRuleTable::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        s.dynamic_rules
            .rules
            .push(rule(".parent:focus .child", "color", "#0000ff"));
        s.nodes[0].focused = true;
        rematch_pseudo_classes(&mut s);
        assert_eq!(
            s.nodes[1].style.color,
            [0.0, 0.0, 1.0, 1.0],
            "parent:focus → child 变蓝"
        );
    }

    #[test]
    fn background_image_change_is_visual_not_layout_dirty() {
        // background-image 是视觉字段（非 taffy_style/order）→ rematch 不 layout dirty
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn:hover", "background-image", "url(icons/home.png)"));
        s.nodes[1].hovered = true;
        let changed = rematch_pseudo_classes(&mut s);
        assert_eq!(
            s.nodes[1].style.background_image.as_deref(),
            Some("icons/home.png"),
            "hover → background-image 生效"
        );
        assert!(!changed, "background-image 视觉字段 → layout 不 dirty");
    }

    #[test]
    fn background_size_change_is_visual_not_layout_dirty() {
        // background-size 是视觉字段 → rematch 不 layout dirty
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn:hover", "background-size", "cover"));
        s.nodes[1].hovered = true;
        let changed = rematch_pseudo_classes(&mut s);
        assert_eq!(
            s.nodes[1].style.background_size,
            crate::style::resolved::BackgroundSize::Cover,
            "hover → background-size:cover 生效"
        );
        assert!(!changed, "background-size 视觉字段 → layout 不 dirty");
    }

    #[test]
    fn border_radius_change_is_visual_not_layout_dirty() {
        // border-radius 是视觉字段（非 taffy_style/order）→ rematch 不 layout dirty
        let mut s = btn_scene();
        s.dynamic_rules
            .rules
            .push(rule(".btn:hover", "border-radius", "8px"));
        s.nodes[1].hovered = true;
        let changed = rematch_pseudo_classes(&mut s);
        // hover → border-radius:8px 生效（四角 h=v=Length(8)）
        let bc = &s.nodes[1].style.border_radius.corners;
        for c in bc {
            assert_eq!(c.h, taffy::style::LengthPercentage::Length(8.0), "hover → border-radius 水平 8px");
            assert_eq!(c.v, taffy::style::LengthPercentage::Length(8.0), "hover → border-radius 垂直 8px");
        }
        assert!(!changed, "border-radius 视觉字段 → layout 不 dirty");
    }
}
