//! Stage 6.7：控件必须被 CSS 命中校验。
//!
//! Yio 控件（ProgressBar / Slider / Toggle / RadioButton / Dropdown /
//! TextField / TextArea / NumberField）**不带 UA 默认样式**——
//! core 刻意保持纯净，不开「框架自带样式源」先例。代价：写了控件标签却没匹配的 CSS
//! 规则 = 运行时渲染空白，作者无法察觉（HTML 在浏览器预览里浏览器会套自己的 UA 表，
//! 看着正常，打包进 Yio 却空）。
//!
//! 本 pass 在打包期（cascade resolve 之后）拦下这种写法：对每个控件节点，检查是否有
//! 任意 `<style>` 规则的选择器命中它本身（tag / class / id / 后代链落地在该节点）。
//! 完全无命中 → `FenceControlWithoutCss` error + 教学。
//!
//! 必需子节点同样校验（同门扩展）：控件命中只证明作者在样式控件本体，不证明子部件
//! 被样式——`data-slot=thumb` 无 background = 可拖不可见的隐形滑块头。按 6.8 契约表
//! （`REQUIRED_CHILDREN` 单一真相源）对每个必需子**实例**查命中，任一无命中 →
//! `FenceControlChildWithoutCss` error（`option`/`listitem` 多实例逐个查——每个列表行
//! 都需要样式，存在一个被命中的不算过）。
//!
//! 控件一律由 `role` 驱动：`<div role="...">`。教学文案按
//! **role/slot** 表述（`data-slot="fill"`、`role="listbox"`、`[aria-checked]` 属性
//! 选择器），不引用任何框架注入的 `.yio-*` 子节点。
//!
//! 选择器匹配消费 fence 的 IrTree（解析期产物），不依赖运行时 Node——复用 css_rules
//! 解析出的 `DynamicRule` 表，按 tag/class/id/attr 字面对照 IrElement 判定。

use crate::diagnostic::{Diagnostic, DiagnosticCode, LineMap};
use crate::ir::{IrElement, IrNodeId, IrNodeKind, IrTree};
use yio_core::style::dynamic::{AttrOp, Combinator, Compound, DynamicRule, ParsedSelector};

/// 触发本校验的控件 role。带这些 role 的元素必被检查；`textbox` 同时覆盖
/// TextField 与 TextArea（后者加 `aria-multiline="true"`）。
const CONTROL_ROLES: &[&str] = &[
    "combobox",
    "slider",
    "spinbutton",
    "switch",
    "radio",
    "progressbar",
    "textbox",
];

/// 读元素的 `role` 属性值（若存在）。
fn node_role(el: &IrElement) -> Option<&str> {
    el.attributes
        .iter()
        .find(|a| a.name == "role")
        .map(|a| a.value.as_str())
}

/// 判定元素是否为受校验控件：`role` 在 CONTROL_ROLES 即是。
fn is_control(el: &IrElement) -> bool {
    node_role(el).is_some_and(|r| CONTROL_ROLES.contains(&r))
}

/// compound（单段选择器，无空格）是否匹配 IrElement——tag/class/id/attr 字面对照。
///
/// 伪类（hover/active/...）不参与：本检查只问「作者是否在样式这个控件」，带状态的规则
/// （`progress:hover{}`）同样表明作者意图——只校 tag/class/id/attr 的静态部分。
fn compound_matches_element(c: &Compound, el: &IrElement) -> bool {
    if let Some(t) = &c.tag {
        if !t.eq_ignore_ascii_case(&el.tag) {
            return false;
        }
    }
    if let Some(id) = &c.id {
        let node_id = el
            .attributes
            .iter()
            .find(|a| a.name == "id")
            .map(|a| a.value.as_str());
        if node_id != Some(id.as_str()) {
            return false;
        }
    }
    if !c.classes.is_empty() {
        let node_classes: Vec<&str> = el
            .attributes
            .iter()
            .find(|a| a.name == "class")
            .map(|a| a.value.split_whitespace().collect())
            .unwrap_or_default();
        for cls in &c.classes {
            if !node_classes.contains(&cls.as_str()) {
                return false;
            }
        }
    }
    for a in &c.attrs {
        let node_attr = el
            .attributes
            .iter()
            .find(|na| na.name.eq_ignore_ascii_case(&a.name));
        match a.op {
            AttrOp::Exists => {
                if node_attr.is_none() {
                    return false;
                }
            }
            AttrOp::Eq => match node_attr {
                Some(na) => {
                    if na.value != a.value.as_deref().unwrap_or("") {
                        return false;
                    }
                }
                None => return false,
            },
        }
    }
    true
}

/// 完整选择器是否命中 node_id：最后一段须命中 node 本身，前面各段沿父链匹配
/// （Child 组合子限直接父——#114；Descendant 沿祖先链逐层尝试）。
pub(crate) fn selector_matches_node(sel: &ParsedSelector, tree: &IrTree, node_idx: usize) -> bool {
    let comps = &sel.compound;
    if comps.is_empty() {
        return false;
    }
    let last = &comps[comps.len() - 1];
    let last_el = match &tree.nodes[node_idx].kind {
        IrNodeKind::Element(e) => e,
        _ => return false,
    };
    if !compound_matches_element(last, last_el) {
        return false;
    }
    if comps.len() == 1 {
        return true;
    }
    match_ancestor_chain(comps, comps.len() - 1, node_idx, tree)
}

/// 递归匹配 comps[0..end_idx] 在 start_node 的父链上（core dynamic.rs
/// `match_chain_with_state` 同款语义，消费 IrTree 而非 Scene）。
/// `start_node` 已命中 comps[end_idx]；为 comps[end_idx-1] 找父：
/// Child=直接父（唯一候选，无回溯）；Descendant=任一祖先，带回溯。
fn match_ancestor_chain(
    comps: &[Compound],
    end_idx: usize,
    start_node: usize,
    tree: &IrTree,
) -> bool {
    if end_idx == 0 {
        return true;
    }
    let target_comp = &comps[end_idx - 1];
    let matched = |ancestor: IrNodeId| {
        matches!(&tree.nodes[ancestor.0].kind, IrNodeKind::Element(anc_el)
            if compound_matches_element(target_comp, anc_el))
            && match_ancestor_chain(comps, end_idx - 1, ancestor.0, tree)
    };
    match comps[end_idx].combinator {
        Combinator::Child => tree.nodes[start_node].parent.is_some_and(matched),
        Combinator::Descendant => {
            let mut cur = tree.nodes[start_node].parent;
            while let Some(ancestor) = cur {
                if matched(ancestor) {
                    return true;
                }
                cur = tree.nodes[ancestor.0].parent;
            }
            false
        }
    }
}

/// 任一规则的选择器命中 node_idx。
fn any_rule_matches(rules: &[DynamicRule], tree: &IrTree, node_idx: usize) -> bool {
    rules
        .iter()
        .any(|r| selector_matches_node(&r.selector, tree, node_idx))
}

/// 规则在某节点上命中且声明了指定 prop=value。
fn any_rule_declares(
    rules: &[DynamicRule],
    tree: &IrTree,
    node_idx: usize,
    prop: &str,
    value: &str,
) -> bool {
    rules.iter().any(|r| {
        selector_matches_node(&r.selector, tree, node_idx)
            && r.declarations
                .iter()
                .any(|d| d.prop == prop && d.value.trim() == value)
    })
}

/// 控件结构 CSS 契约表：控件运行时行为对作者 CSS 的**结构性**依赖。
///
/// 与「命中校验」（任何规则命中即可）互补：命中只证明作者在样式这个控件，
/// 不证明结构声明齐全。缺结构声明的症状在 PlayMode 才可见（弹层撑开容器 /
/// 定位飞出），正是围栏「不静默降级」要打包期拦截的类别。
///
/// 每条契约：控件本体必需声明（如锚点 position:relative）+ 一个弹层后代角色
/// 的必需声明（如 position:absolute 脱流）。后续控件（slider fill/thumb 绝对
/// 定位等）在契约核实后于本表扩展，勿散落硬编码。
struct StructureCssContract {
    /// 控件 role（字面值，与 REQUIRED_CHILDREN 同口径）。
    role: &'static str,
    /// 控件本体必需声明 (prop, value)。
    control_decl: (&'static str, &'static str),
    /// 弹层后代 role（子树内递归找）。
    popup_role: &'static str,
    /// 弹层必需声明 (prop, value)。
    popup_decl: (&'static str, &'static str),
    /// 教学文案：标准写法（可直接抄的 CSS）。
    canonical: &'static str,
}

const STRUCTURE_CSS_CONTRACTS: &[StructureCssContract] = &[StructureCssContract {
    role: "combobox",
    control_decl: ("position", "relative"),
    popup_role: "listbox",
    popup_decl: ("position", "absolute"),
    canonical: "[role=\"combobox\"] { position:relative; }\n\
                [role=\"combobox\"] [role=\"listbox\"] { display:none; position:absolute; \
                left:0; top:100%; width:100%; }",
}];

/// 递归找子树内带指定 role 的首个元素节点索引。
fn find_descendant_by_role(tree: &IrTree, root_idx: usize, role: &str) -> Option<usize> {
    for &child in &tree.nodes[root_idx].children {
        if let IrNodeKind::Element(el) = &tree.nodes[child.0].kind {
            if node_role(el) == Some(role) {
                return Some(child.0);
            }
        }
        if let Some(found) = find_descendant_by_role(tree, child.0, role) {
            return Some(found);
        }
    }
    None
}

/// Stage 6.7b：控件结构 CSS 契约校验。表驱动（[`STRUCTURE_CSS_CONTRACTS`]）。
///
/// 对每条契约的每个控件节点：本体必需声明 + 弹层后代必需声明，任一缺失即
/// `FenceControlStructureCss` error。只在 Annotate + Stage 4.5 之后跑（同
/// [`check_control_css`] 的前置）。
pub fn check_control_structure_css(
    tree: &IrTree,
    dynamic_rules: &[DynamicRule],
    file: &str,
    line_map: &LineMap,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (idx, node) in tree.nodes.iter().enumerate() {
        let IrNodeKind::Element(el) = &node.kind else {
            continue;
        };
        let role = match node_role(el) {
            Some(r) => r,
            None => continue,
        };
        let Some(contract) = STRUCTURE_CSS_CONTRACTS.iter().find(|c| c.role == role) else {
            continue;
        };
        if !any_rule_declares(
            dynamic_rules,
            tree,
            idx,
            contract.control_decl.0,
            contract.control_decl.1,
        ) {
            let loc = line_map.source_location(node.span.start, file.to_string());
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::FenceControlStructureCss,
                format!(
                    "Yio {role} control is missing `{}`:{} — the popup anchor. \
                     Without it the popup positions against an outer containing block and \
                     the viewport-flip placement breaks. Canonical form:\n{}",
                    contract.control_decl.0, contract.control_decl.1, contract.canonical
                ),
                loc,
            ));
        }
        if let Some(popup_idx) = find_descendant_by_role(tree, idx, contract.popup_role) {
            if !any_rule_declares(
                dynamic_rules,
                tree,
                popup_idx,
                contract.popup_decl.0,
                contract.popup_decl.1,
            ) {
                let popup_node = &tree.nodes[popup_idx];
                let loc = line_map.source_location(popup_node.span.start, file.to_string());
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::FenceControlStructureCss,
                    format!(
                        "Yio {role} popup (`role=\"{}\"`) is missing `{}:{}` — \
                         without it the popup stays in flow and expands its container \
                         when opened. Canonical form:\n{}",
                        contract.popup_role,
                        contract.popup_decl.0,
                        contract.popup_decl.1,
                        contract.canonical
                    ),
                    loc,
                ));
            }
        }
    }
    diagnostics
}

/// thumb 上被控件忽略的定位属性（prop 名字面量）。`position:absolute` 本身不在列——
/// thumb 需要脱流锚定（showcase 标准写法 `left:0; top:0` 会被归零为 0，语义等价）。
const THUMB_POSITIONED_PROPS: &[&str] = &[
    "top",
    "right",
    "bottom",
    "left",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
];

/// Stage 6.7c：slider thumb 定位所有权校验（warning）。
///
/// thumb 的位移由控件运行时按 value 全权驱动（水平位移 + 垂直居中，core 每帧把
/// thumb 的 inset/margin 归零再写 transform）。作者 CSS 给 thumb 写定位（负 `top`
/// 居中、`left` 百分比等浏览器直觉写法）不生效且与控件位移叠加会双偏移——本检查
/// 在打包期提示所有权，避免「浏览器预览居中、运行时偏移」的静默分歧。尺寸与
/// 外观声明不受影响。
pub fn check_slider_thumb_positioning(
    tree: &IrTree,
    dynamic_rules: &[DynamicRule],
    file: &str,
    line_map: &LineMap,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (idx, node) in tree.nodes.iter().enumerate() {
        let IrNodeKind::Element(el) = &node.kind else {
            continue;
        };
        let is_thumb = el
            .attributes
            .iter()
            .any(|a| a.name == "data-slot" && a.value == "thumb");
        if !is_thumb {
            continue;
        }
        for rule in dynamic_rules {
            if !selector_matches_node(&rule.selector, tree, idx) {
                continue;
            }
            let offenders: Vec<&str> = rule
                .declarations
                .iter()
                .filter_map(|d| {
                    let is_zero = matches!(d.value.trim(), "0" | "0px" | "0%");
                    THUMB_POSITIONED_PROPS
                        .iter()
                        .find(|p| d.prop.eq_ignore_ascii_case(p) && !is_zero)
                })
                .copied()
                .collect();
            if offenders.is_empty() {
                continue;
            }
            diagnostics.push(Diagnostic::warning(
                DiagnosticCode::FenceSliderThumbPositioned,
                format!(
                    "Slider thumb (`data-slot=\"thumb\"`) declares positioning ({}) — \
                     the control owns thumb placement: runtime drives horizontal offset by \
                     value and centers it vertically, zeroing inset/margin every frame. \
                     Author positioning silently shifts (browser preview centers, runtime \
                     is offset twice). Keep size/appearance here; for placement write \
                     `left:0; top:0` (the canonical anchor) or nothing at all.",
                    offenders.join(", "),
                ),
                line_map.source_location(node.span.start, file.to_string()),
            ));
        }
    }
    diagnostics
}

/// 控件的可读名称（教学文案用）。按 `role` 取名。
fn kind_name_for(el: &IrElement) -> &'static str {
    match node_role(el) {
        Some("combobox") => "dropdown (combobox)",
        Some("slider") => "slider",
        Some("spinbutton") => "number field (spinbutton)",
        Some("switch") => "toggle (switch)",
        Some("radio") => "radio button",
        Some("progressbar") => "progress bar",
        Some("textbox") => "text field",
        _ => "control",
    }
}

/// 按控件生成「该怎么配 CSS」教学文案（role/slot 表述）。
fn fix_hint_for(el: &IrElement) -> String {
    let tag = el.tag.as_str();
    match node_role(el) {
        Some("progressbar") => format!(
            "Provide CSS for <{tag}> (the track — e.g. a background/border) and for its \
             `data-slot=\"fill\"` child (the fill bar). Both elements need CSS; without it \
             the progress bar renders blank."
        ),
        Some("slider") => format!(
            "Provide CSS for <{tag}> (the track — e.g. a background/border) and for its \
             `data-slot=\"thumb\"` child (the draggable handle). A `data-slot=\"fill\"` \
             child is optional for the filled portion. All present elements need CSS."
        ),
        Some("combobox") => format!(
            "Provide CSS for <{tag}> (background/border so the box is visible), for its \
             `role=\"listbox\"` child (the popup list container), and for `role=\"option\"` \
             children (each list row). Yio dropdowns have NO built-in arrow indicator — \
             if you want one, draw it yourself via CSS (e.g. a background-image on the box, \
             or an extra child element)."
        ),
        Some("switch") | Some("radio") => format!(
            "Provide CSS for <{tag}> (background/border so the control is visible). Use the \
             `[aria-checked]` attribute selector to style checked/unchecked states — there is \
             no separate check-mark child element."
        ),
        Some("textbox") => format!(
            "Provide CSS for <{tag}> (background/border and caret-color so the text field is \
             visible). Add `aria-multiline=\"true\"` for a multi-line text area."
        ),
        Some("spinbutton") => format!(
            "Provide CSS for <{tag}> (background/border and caret-color so the number field is \
             visible)."
        ),
        _ => format!("Provide CSS for <{tag}> so the control is visible."),
    }
}

/// 检查所有控件节点是否被至少一条 CSS 规则命中；必需子节点（6.8 契约表）的每个
/// 实例同样须被命中。返回诊断（error 列表）。
///
/// 入参：
/// - `tree`：IrTree（已过 Annotate，`IrElement.semantic` 已填充）
/// - `dynamic_rules`：Stage 4.5 解析出的 `<style>` 规则表
/// - `file` / `line_map`：定位诊断
pub fn check_control_css(
    tree: &IrTree,
    dynamic_rules: &[DynamicRule],
    file: &str,
    line_map: &LineMap,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (idx, node) in tree.nodes.iter().enumerate() {
        let IrNodeKind::Element(el) = &node.kind else {
            continue;
        };
        if !is_control(el) {
            continue;
        }
        if any_rule_matches(dynamic_rules, tree, idx) {
            continue;
        }

        let tag = el.tag.as_str();
        let kind_name = kind_name_for(el);
        let fix_hint = fix_hint_for(el);
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::FenceControlWithoutCss,
            format!(
                "Yio {kind_name} element <{tag}> has no matching CSS rule. \
                 Controls have NO built-in default style — without CSS they render blank. \
                 {fix_hint} Canonical control CSS: `patterns.md` in the scaffolded \
                 yio-editor skill."
            ),
            line_map.source_location(node.span.start, file.to_string()),
        ));
    }
    diagnostics.extend(check_required_child_css(
        tree,
        dynamic_rules,
        file,
        line_map,
    ));
    diagnostics
}

/// 必需子节点 CSS 命中校验（6.7 门扩展）：按 6.8 契约表（`REQUIRED_CHILDREN`
/// 单一真相源）对每个必需子**实例**查命中。
///
/// 本体命中（上面的循环）只证明作者在样式控件本体，不证明子部件被样式——
/// `data-slot=thumb` 无 background = 可拖不可见的隐形滑块头，`role=option` 无 CSS
/// = 弹层里的隐形行。`option`/`listitem` 多实例逐个查（每个列表行都需要样式，
/// 存在一个被命中的不算过）；template 蓝图内的 listitem 同样查（蓝图无 CSS，
/// 克隆体也无）。控件集合 = 契约表全集（含 list/tablist/listbox——比本体命中
/// 校验的 CONTROL_ROLES 宽：它们本体无自绘样式不查本体，但必需子要查）。
fn check_required_child_css(
    tree: &IrTree,
    dynamic_rules: &[DynamicRule],
    file: &str,
    line_map: &LineMap,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (idx, node) in tree.nodes.iter().enumerate() {
        let IrNodeKind::Element(el) = &node.kind else {
            continue;
        };
        let Some(role) = node_role(el) else {
            continue;
        };
        let Some((_, specs)) = crate::control_structure_check::REQUIRED_CHILDREN
            .iter()
            .find(|(r, _)| *r == role)
        else {
            continue;
        };
        for &spec in *specs {
            for child_idx in
                crate::control_structure_check::required_child_instances(tree, idx, spec)
            {
                if any_rule_matches(dynamic_rules, tree, child_idx) {
                    continue;
                }
                let child = &tree.nodes[child_idx];
                let IrNodeKind::Element(child_el) = &child.kind else {
                    continue;
                };
                let parent_tag = el.tag.as_str();
                let label = match spec {
                    crate::control_structure_check::CheckSpec::Role(r) => {
                        format!("role=\"{r}\"")
                    }
                    crate::control_structure_check::CheckSpec::Slot(s) => {
                        format!("data-slot=\"{s}\"")
                    }
                };
                let child_tag = child_el.tag.as_str();
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::FenceControlChildWithoutCss,
                    format!(
                        "Yio <{parent_tag} role=\"{role}\"> has a required \
                         <{child_tag} {label}> child with no matching CSS rule. \
                         Control children have NO built-in default style — without CSS \
                         they render invisible (e.g. a thumb without background is a \
                         draggable-but-invisible handle). Provide CSS for every \
                         {label} child. Canonical control CSS: `patterns.md` in the \
                         scaffolded yio-editor skill."
                    ),
                    line_map.source_location(child.span.start, file.to_string()),
                ));
            }
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::parse_template;

    /// 辅助：解析 HTML 后跑本检查（隔离单元测试，不经 pipeline 全程）。
    fn check(html: &str) -> Vec<Diagnostic> {
        let result = parse_template(html, "t.html");
        // 只取本检查产出的诊断（过滤掉其他 stage 的噪声，如 inline-context）。
        check_control_css(
            &result.tree,
            &result.dynamic_rules,
            "t.html",
            &crate::diagnostic::LineMap::new(html),
        )
    }

    /// 辅助：同 check，跑结构契约检查（Stage 6.7b）。
    fn check_structure(html: &str) -> Vec<Diagnostic> {
        let result = parse_template(html, "t.html");
        check_control_structure_css(
            &result.tree,
            &result.dynamic_rules,
            "t.html",
            &crate::diagnostic::LineMap::new(html),
        )
    }

    /// 辅助：同 check，跑 thumb 定位所有权检查（Stage 6.7c）。
    fn check_thumb_pos(html: &str) -> Vec<Diagnostic> {
        let result = parse_template(html, "t.html");
        check_slider_thumb_positioning(
            &result.tree,
            &result.dynamic_rules,
            "t.html",
            &crate::diagnostic::LineMap::new(html),
        )
    }

    #[test]
    fn thumb_nonzero_positioning_warns() {
        // 浏览器直觉写法（负 top 居中 / left 百分比 / margin 微调）在 thumb 上与控件
        // 位移叠加双偏移——warning 提示所有权。
        let diags = check_thumb_pos(
            "<div><div role=\"slider\"><div data-slot=\"thumb\"></div></div></div>\
             <style>[role=slider] [data-slot=thumb]{top:-9px;left:62%;margin-top:-12px;\
             width:24px;height:24px}</style>",
        );
        assert_eq!(diags.len(), 1, "diags: {diags:?}");
        assert_eq!(diags[0].code, DiagnosticCode::FenceSliderThumbPositioned);
        assert!(diags[0].message.contains("top"), "offender 应列出 prop");
    }

    #[test]
    fn thumb_canonical_zero_anchor_and_size_silent() {
        // 标准写法（left:0; top:0 锚定 + 尺寸/外观）零告警——归零后语义等价。
        let diags = check_thumb_pos(
            "<div><div role=\"slider\"><div data-slot=\"thumb\"></div></div></div>\
             <style>[role=slider] [data-slot=thumb]{position:absolute;left:0;top:0;\
             width:16px;height:16px;border-radius:8px}</style>",
        );
        assert!(diags.is_empty(), "diags: {diags:?}");
    }

    #[test]
    fn thumb_positioning_on_unrelated_element_silent() {
        // 非 thumb 元素的定位声明不受本检查约束。
        let diags = check_thumb_pos(
            "<div><div role=\"slider\"><div data-slot=\"thumb\"></div></div>\
             <div class=\"badge\"></div></div>\
             <style>[role=slider] [data-slot=thumb]{left:0;top:0}\
             .badge{top:-9px}</style>",
        );
        assert!(diags.is_empty(), "diags: {diags:?}");
    }

    #[test]
    fn structure_contract_compliant_combobox_passes() {
        // form/settings 页的标准写法：锚点 + 脱流 + 隐藏初态全齐 → 零诊断。
        let diags = check_structure(
            "<div><div role=\"combobox\"><div role=\"listbox\">\
             <div role=\"option\">a</div></div></div></div>\
             <style>[role=combobox]{position:relative} \
             [role=combobox] [role=listbox]{display:none;position:absolute}</style>",
        );
        assert!(diags.is_empty(), "diags: {diags:?}");
    }

    #[test]
    fn structure_contract_missing_both_declarations_errors() {
        // 视觉规则命中但结构声明全缺 → 两条 error
        //（锚点 + 脱流）。「命中校验」（6.7）对此放行——本检查补位。
        let diags = check_structure(
            "<div><div role=\"combobox\"><div role=\"listbox\">\
             <div role=\"option\">a</div></div></div></div>\
             <style>[role=combobox]{background-color:#101c28;width:280px}</style>",
        );
        assert_eq!(diags.len(), 2, "diags: {diags:?}");
        assert!(diags
            .iter()
            .all(|d| d.code == DiagnosticCode::FenceControlStructureCss));
    }

    #[test]
    fn structure_contract_popup_absolute_without_anchor_errors_once() {
        // 弹层脱流了但控件本体没锚点 → 单条锚点 error。
        let diags = check_structure(
            "<div><div role=\"combobox\"><div role=\"listbox\">\
             <div role=\"option\">a</div></div></div></div>\
             <style>[role=combobox] [role=listbox]{position:absolute}</style>",
        );
        assert_eq!(diags.len(), 1, "diags: {diags:?}");
        assert!(diags[0].message.contains("anchor"));
    }

    #[test]
    fn structure_contract_non_combobox_controls_untouched() {
        // 契约表外的控件（slider 等）不受本检查影响（契约核实后再入表）。
        let diags = check_structure(
            "<div><div role=\"slider\"><div data-slot=\"thumb\"></div></div></div>\
             <style>[role=slider]{width:100px}</style>",
        );
        assert!(diags.is_empty(), "diags: {diags:?}");
    }

    #[test]
    fn compound_matches_by_tag() {
        let mut el = IrElement {
            tag: "div".into(),
            attributes: vec![],
            semantic: None,
        };
        let c = parse_compound("div");
        assert!(compound_matches_element(&c, &el));
        el.tag = "span".into();
        assert!(!compound_matches_element(&c, &el));
    }

    #[test]
    fn compound_matches_by_class() {
        let el = IrElement {
            tag: "div".into(),
            attributes: vec![attr("class", "hp big")],
            semantic: None,
        };
        assert!(compound_matches_element(&parse_compound(".hp"), &el));
        assert!(compound_matches_element(&parse_compound(".big"), &el));
        assert!(!compound_matches_element(&parse_compound(".x"), &el));
    }

    #[test]
    fn compound_matches_by_attr_eq() {
        let el = IrElement {
            tag: "div".into(),
            attributes: vec![attr("role", "slider")],
            semantic: None,
        };
        assert!(compound_matches_element(
            &parse_compound(r#"div[role="slider"]"#),
            &el
        ));
        assert!(!compound_matches_element(
            &parse_compound(r#"div[role="switch"]"#),
            &el
        ));
    }

    #[test]
    fn role_progressbar_without_css_errors() {
        let diags = check(r#"<div role="progressbar"></div>"#);
        assert_eq!(diags.len(), 1);
        // 文案应引导 data-slot="fill"（不再引用已删除的 .yio-fill）
        assert!(
            diags[0].message.contains("data-slot=\"fill\""),
            "{}",
            diags[0].message
        );
        assert!(
            !diags[0].message.contains(".yio-"),
            "不应再引用 .yio-*: {}",
            diags[0].message
        );
    }

    #[test]
    fn role_slider_without_css_errors() {
        let diags = check(r#"<div role="slider"></div>"#);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message.contains("data-slot=\"thumb\""),
            "{}",
            diags[0].message
        );
        assert!(!diags[0].message.contains(".yio-"), "{}", diags[0].message);
    }

    #[test]
    fn role_combobox_without_css_errors() {
        let diags = check(
            r#"<div role="combobox"><div role="listbox"><div role="option">A</div></div></div>"#,
        );
        // 3 条：本体（FenceControlWithoutCss）+ listbox 子 + option 子（均无 CSS，
        // 新必需子命中校验）。文案应引导 role=listbox / role=option + 仍含
        // 「NO built-in arrow」教学点。
        assert_eq!(diags.len(), 3, "{diags:?}");
        assert!(
            diags[0].message.contains("role=\"listbox\""),
            "{}",
            diags[0].message
        );
        assert!(
            diags[0].message.contains("NO built-in arrow"),
            "{}",
            diags[0].message
        );
        assert!(!diags[0].message.contains(".yio-"), "{}", diags[0].message);
        assert_eq!(diags[1].code, DiagnosticCode::FenceControlChildWithoutCss);
        assert_eq!(diags[2].code, DiagnosticCode::FenceControlChildWithoutCss);
    }

    #[test]
    fn role_switch_without_css_errors() {
        let diags = check(r#"<div role="switch"></div>"#);
        assert_eq!(diags.len(), 1);
        // switch / radio 无必需子节点：文案应引导 [aria-checked] 属性选择器
        assert!(
            diags[0].message.contains("[aria-checked]"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn role_textbox_without_css_errors() {
        let diags = check(r#"<div role="textbox"></div>"#);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message.contains("caret-color"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn role_control_with_matching_attr_selector_passes() {
        // [role="slider"] 属性选择器命中 role 驱动控件 → 放行
        let diags =
            check(r#"<style>[role="slider"]{background:#ddd}</style><div role="slider"></div>"#);
        assert!(diags.is_empty(), "{diags:?}");
    }

    fn attr(name: &str, value: &str) -> crate::ir::IrAttribute {
        crate::ir::IrAttribute {
            name: name.into(),
            value: value.into(),
            span: crate::ir::Span::default(),
        }
    }

    fn parse_compound(raw: &str) -> Compound {
        // parse_selector 产 ParsedSelector；单 compound 取 [0]。
        let sel = crate::css_rules::parse_selector(raw).unwrap_or_else(|| panic!("parse {raw:?}"));
        sel.compound.into_iter().next().expect("one compound")
    }
}
