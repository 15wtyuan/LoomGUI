use crate::parse::css::Rule;
use crate::parse::dom::{ElementData, ElementId, ElementTree};

// 选择器数据模型（ParsedSelector/Compound/Combinator/Specificity）+ compound_matches_node
// 定义在常驻模块 `style::dynamic`（bincode 序列化进 .pkg.bin，runtime 不依赖 parse feature）。
// 本 parse-gated 模块只提供解析器函数（string → 这些结构）+ ElementTree 版匹配。
pub use crate::style::dynamic::{
    compound_matches_node, Combinator, Compound, ParsedSelector, Specificity,
};

/// 极简解析：按空格切 descendant，`>` 切 child；复合内 tag/.class/#id。
pub fn parse_selector(raw: &str) -> Result<ParsedSelector, String> {
    let raw_trimmed = raw.trim().to_string();

    // 第一遍：按 `>` / 空格切 compound，记录每个 compound 与前一个的 combinator。
    // 第一个 compound 的 combinator 字段无意义（前面没有节点）。
    let mut parts: Vec<(String, Combinator)> = Vec::new();
    let mut buf = String::new();
    let mut next_comb = Combinator::Descendant;
    let mut first = true;
    for ch in raw_trimmed.chars() {
        match ch {
            '>' => {
                if !buf.trim().is_empty() {
                    parts.push((
                        buf.trim().to_string(),
                        if first {
                            Combinator::Descendant
                        } else {
                            next_comb
                        },
                    ));
                    first = false;
                }
                buf.clear();
                next_comb = Combinator::Child;
            }
            c if c.is_whitespace() => {
                if !buf.trim().is_empty() {
                    parts.push((
                        buf.trim().to_string(),
                        if first {
                            Combinator::Descendant
                        } else {
                            next_comb
                        },
                    ));
                    first = false;
                    buf.clear();
                }
                // 空格降级为 descendant，但不要覆盖 `>` 已设的 child。
                // （CSS 里 `a > b` 的 `>` 两侧可有可无空格，空格不能把 child 降回 descendant。）
                if next_comb != Combinator::Child {
                    next_comb = Combinator::Descendant;
                }
            }
            c => {
                buf.push(c);
            }
        }
    }
    if !buf.trim().is_empty() {
        parts.push((
            buf.trim().to_string(),
            if first {
                Combinator::Descendant
            } else {
                next_comb
            },
        ));
    }

    // 第二遍：每个 compound 文本内按 `.` / `#` 切 tag/class/id；累加 specificity。
    let mut spec = Specificity(0, 0, 0);
    let mut compound: Vec<Compound> = Vec::new();
    for (text, comb) in &parts {
        let mut tag: Option<String> = None;
        let mut classes: Vec<String> = Vec::new();
        let mut id: Option<String> = None;

        let bytes = text.as_bytes();
        let mut idx = 0;
        let mut kind = 't'; // t=tag, .=class, #=id
        let mut cur = String::new();
        let push_token = |kind: char,
                              val: &str,
                              tag: &mut Option<String>,
                              classes: &mut Vec<String>,
                              id: &mut Option<String>| {
            if val.is_empty() {
                return;
            }
            match kind {
                't' => *tag = Some(val.to_string()),
                '.' => classes.push(val.to_string()),
                '#' => *id = Some(val.to_string()),
                _ => {}
            }
        };
        while idx < bytes.len() {
            let c = bytes[idx] as char;
            if c == '.' || c == '#' || c == ':' {
                push_token(kind, &cur, &mut tag, &mut classes, &mut id);
                cur.clear();
                kind = c;
            } else {
                cur.push(c);
            }
            idx += 1;
        }
        push_token(kind, &cur, &mut tag, &mut classes, &mut id);

        if id.is_some() {
            spec.0 += 1;
        }
        spec.1 += classes.len() as u32;
        if tag.is_some() {
            spec.2 += 1;
        }
        let mut pseudo_hover = false;
        let mut pseudo_active = false;
        let mut pseudo_disabled = false;
        let mut pseudo_focus = false;
        let mut rest = text.as_str();
        while let Some(colon) = rest.find(':') {
            let after = &rest[colon + 1..];
            let end = after
                .find(|c: char| c == '.' || c == '#' || c == ':')
                .unwrap_or(after.len());
            let name = &after[..end];
            match name {
                "hover" => pseudo_hover = true,
                "active" => pseudo_active = true,
                "disabled" => pseudo_disabled = true,
                "focus" => pseudo_focus = true,
                _ => {} // 未知伪类静默忽略（v1d.2 认 hover/active/disabled/focus）
            }
            rest = &after[end..];
        }
        compound.push(Compound {
            tag,
            classes,
            id,
            combinator: *comb,
            pseudo_hover,
            pseudo_active,
            pseudo_disabled,
            pseudo_focus,
        });
    }

    Ok(ParsedSelector {
        raw: raw_trimmed,
        compound,
        specificity: spec,
    })
}

fn compound_matches(c: &Compound, el: &ElementData) -> bool {
    // 伪类规则不参与 base cascade——运行时由 rematch_pseudo_classes + match_element_with_state
    // 按节点 hovered/active/disabled/focused 状态动态应用。base 只烤静态规则（与
    // extract_dynamic_rules 配套：伪类规则进 DynamicRuleSection）。
    // 坑：漏检会让 .btn:focus 紫污染 .btn base（specificity 同级源序后胜 → 全 .btn 紫）。
    if c.pseudo_hover || c.pseudo_active || c.pseudo_disabled || c.pseudo_focus {
        return false;
    }
    if let Some(t) = &c.tag {
        if el.tag != *t {
            return false;
        }
    }
    if let Some(id) = &c.id {
        if el.id.as_deref() != Some(id.as_str()) {
            return false;
        }
    }
    for cls in &c.classes {
        if !el.classes.iter().any(|c| c == cls) {
            return false;
        }
    }
    true
}

/// 从右往左匹配：最后一个 compound 必须命中目标元素，前面的按 combinator 沿父链找。
///
/// 标准 CSS 后代/子代语义：
/// - 最后一个 compound：必须命中目标元素本身
/// - 往左每个 compound，按它的 `combinator`（与它右边 compound 的关系）：
///   - `Combinator::Child`：必须在**直接父**匹配
///   - `Combinator::Descendant`：必须在**任一祖先**匹配（沿 parent 链向上搜）
///
/// 后代匹配用回溯：在祖先链上找到任一满足该 compound 的祖先后继续往左匹配剩余 compounds；
/// 若后续失败，回溯到该祖先的更上层继续尝试。这样 `div.a span` 能在
/// `<div class=a><div><span>` 上命中（div.a 是 span 的祖父）。
fn matches(sel: &ParsedSelector, el_id: ElementId, tree: &ElementTree) -> bool {
    let comps = &sel.compound;
    if comps.is_empty() {
        return false;
    }
    // 最后一个 compound 必须命中目标元素
    let last = &comps[comps.len() - 1];
    if !compound_matches(last, &tree.nodes[el_id.0]) {
        return false;
    }
    if comps.len() == 1 {
        return true;
    }
    // 从目标元素往上，逐个匹配剩余 compounds（从倒数第二个到第一个）
    match_compound_chain(comps, comps.len() - 1, el_id, tree)
}

/// 递归匹配 compounds[0..end_idx]（不含 end_idx），要求这一段链能在 `start_el`
/// 的祖先链上找到对应位置。`start_el` 是已经匹配了 comps[end_idx] 的元素，
/// 我们要为 comps[end_idx - 1] 找一个（Child：直接父；Descendant：任一祖先）匹配的祖先，
/// 再继续为 comps[0..end_idx-1] 在该祖先之上匹配。
fn match_compound_chain(
    comps: &[Compound],
    end_idx: usize,
    start_el: ElementId,
    tree: &ElementTree,
) -> bool {
    if end_idx == 0 {
        return true;
    }
    let target_comp = &comps[end_idx - 1];
    match target_combinator(&comps[end_idx]) {
        Combinator::Child => {
            // Child：必须是直接父
            match tree.nodes[start_el.0].parent {
                Some(parent) => {
                    compound_matches(target_comp, &tree.nodes[parent.0])
                        && match_compound_chain(comps, end_idx - 1, parent, tree)
                }
                None => false,
            }
        }
        Combinator::Descendant => {
            // Descendant：沿祖先链向上找任一匹配的祖先；找到后递归匹配剩余，
            // 若剩余失败则继续往上找下一个匹配祖先（回溯）。
            let mut cur = tree.nodes[start_el.0].parent;
            while let Some(ancestor) = cur {
                if compound_matches(target_comp, &tree.nodes[ancestor.0]) {
                    if match_compound_chain(comps, end_idx - 1, ancestor, tree) {
                        return true;
                    }
                    // 此祖先匹配但更左的链匹配不上 → 继续往上找
                }
                cur = tree.nodes[ancestor.0].parent;
            }
            false
        }
    }
}

/// 取一个 compound 的 combinator 字段。第一个 compound（idx=0）的 combinator
/// 在 parse 阶段被设为 Descendant 占位，但语义上它无前驱，调用方应保证不会走到这里。
/// 这里只是直读字段——对 idx>0 的 compound，字段记录的就是它与前一个 compound 的关系，
/// 也就是「它的右边 compound 通过什么关系到达它」，正是从右往左匹配时需要的关系。
fn target_combinator(c: &Compound) -> Combinator {
    c.combinator
}

/// 给定元素 + 规则集 → 命中规则（已按 specificity 降序、同级按出现顺序）
pub fn match_element<'a>(
    el: &ElementData,
    tree: &'a ElementTree,
    rules: &'a [Rule],
) -> Vec<&'a Rule> {
    // 用 ptr::eq 找目标元素 id。
    // 注：ElementData 是 Clone 的，ptr 比较仅在 el 是 tree.nodes 的直接借用时稳定。
    // 当前调用方（含测试）都传 &tree.nodes[i]，故稳定。若后续改为 clone 后传入会失效，
    // 届时把签名改成收 el_id: ElementId 更稳。
    let el_id = tree
        .nodes
        .iter()
        .position(|n| std::ptr::eq(n, el))
        .map(ElementId);
    let el_id = match el_id {
        Some(id) => id,
        None => return Vec::new(),
    };

    let mut matched: Vec<(Specificity, usize, &Rule)> = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        let sel = match parse_selector(&rule.selector_text) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if matches(&sel, el_id, tree) {
            matched.push((sel.specificity, i, rule));
        }
    }
    // specificity 降序，同级按出现顺序（i 升序）
    matched.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    matched.into_iter().map(|(_, _, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::css::Rule;
    use crate::parse::dom::{ElementId, ElementTree};

    fn el(tag: &str, classes: &[&str], id: Option<&str>) -> ElementData {
        ElementData {
            tag: tag.into(),
            classes: classes.iter().map(|s| s.to_string()).collect(),
            id: id.map(|s| s.into()),
            attrs: std::collections::HashMap::new(),
            text: None,
            children: Vec::new(),
            parent: None,
        }
    }

    #[test]
    fn specificity_tag_class_id() {
        assert_eq!(
            parse_selector("div").unwrap().specificity,
            Specificity(0, 0, 1)
        );
        assert_eq!(
            parse_selector(".a").unwrap().specificity,
            Specificity(0, 1, 0)
        );
        assert_eq!(
            parse_selector("#x").unwrap().specificity,
            Specificity(1, 0, 0)
        );
        assert_eq!(
            parse_selector("div.a#x").unwrap().specificity,
            Specificity(1, 1, 1)
        );
    }

    #[test]
    fn matches_tag_class_id() {
        // 用一棵单元素树测匹配
        let tree = ElementTree {
            roots: vec![ElementId(0)],
            nodes: vec![el("div", &["panel"], Some("main"))],
        };
        let rules = vec![
            Rule {
                selector_text: "div".into(),
                declarations: vec![],
            },
            Rule {
                selector_text: ".panel".into(),
                declarations: vec![],
            },
            Rule {
                selector_text: "#main".into(),
                declarations: vec![],
            },
            Rule {
                selector_text: "span".into(),
                declarations: vec![],
            },
        ];
        let matched = match_element(&tree.nodes[0], &tree, &rules);
        assert_eq!(matched.len(), 3); // div, .panel, #main 命中；span 不中
    }

    /// 后代选择器跨多层匹配：`div.a span` 应在 `<div class=a><div><span>` 上命中 span。
    /// （修复前：matches 只查直接父，div.a 是 span 的祖父 → 静默失败。）
    #[test]
    fn descendant_matches_across_layers() {
        // nodes: 0=div.a (root), 1=div, 2=span
        let mut nodes = vec![
            el("div", &["a"], None),
            el("div", &[], None),
            el("span", &[], None),
        ];
        nodes[1].parent = Some(ElementId(0));
        nodes[2].parent = Some(ElementId(1));
        nodes[0].children = vec![ElementId(1)];
        nodes[1].children = vec![ElementId(2)];
        let tree = ElementTree {
            roots: vec![ElementId(0)],
            nodes,
        };
        let rules = vec![Rule {
            selector_text: "div.a span".into(),
            declarations: vec![],
        }];
        // span 是 nodes[2]
        let matched = match_element(&tree.nodes[2], &tree, &rules);
        assert_eq!(matched.len(), 1, "div.a span 必须命中跨层的 span");
    }

    /// 子代选择器要求直接父匹配。`div.a > span`：span 的直接父是普通 div（nodes[1]），
    /// 而 `div.a` 只是祖父（nodes[0]）。子代要求直接父命中 `div.a` → 不应命中。
    /// （对比：`div.a span` 后代会命中——见上一个测试的同一棵树。）
    #[test]
    fn parse_hover_pseudo() {
        let s = parse_selector(".btn:hover").unwrap();
        assert_eq!(s.compound.len(), 1);
        assert!(s.compound[0].pseudo_hover, ":hover → pseudo_hover=true");
        assert!(!s.compound[0].pseudo_active);
        assert!(!s.compound[0].pseudo_disabled);
        assert_eq!(s.compound[0].classes, vec!["btn".to_string()]);
    }

    #[test]
    fn parse_multiple_pseudos() {
        let s = parse_selector(".btn:hover:active").unwrap();
        assert!(s.compound[0].pseudo_hover);
        assert!(s.compound[0].pseudo_active);
        assert!(!s.compound[0].pseudo_disabled);
    }

    #[test]
    fn parse_disabled_pseudo() {
        let s = parse_selector("button:disabled").unwrap();
        assert!(s.compound[0].pseudo_disabled);
        assert_eq!(s.compound[0].tag.as_deref(), Some("button"));
    }

    #[test]
    fn parse_no_pseudo_defaults_false() {
        let s = parse_selector(".btn").unwrap();
        assert!(!s.compound[0].pseudo_hover);
        assert!(!s.compound[0].pseudo_active);
        assert!(!s.compound[0].pseudo_disabled);
    }

    #[test]
    fn parse_descendant_with_pseudo() {
        let s = parse_selector(".parent:hover .child").unwrap();
        assert_eq!(s.compound.len(), 2);
        assert!(s.compound[0].pseudo_hover, "parent compound 含 :hover");
        assert!(!s.compound[1].pseudo_hover, "child compound 无 :hover");
        assert_eq!(s.compound[1].classes, vec!["child".to_string()]);
    }

    #[test]
    fn parse_focus_pseudo() {
        let s = parse_selector(".btn:focus").unwrap();
        assert!(s.compound[0].pseudo_focus, ":focus → pseudo_focus=true");
        assert!(!s.compound[0].pseudo_hover);
    }

    #[test]
    fn parse_focus_with_hover() {
        let s = parse_selector(".btn:focus:hover").unwrap();
        assert!(s.compound[0].pseudo_focus, ":focus → pseudo_focus");
        assert!(s.compound[0].pseudo_hover, ":hover → pseudo_hover");
    }

    #[test]
    fn parsed_selector_bincode_roundtrip() {
        let s = parse_selector(".btn:hover:active .child").unwrap();
        let bytes = bincode::serialize(&s).unwrap();
        let back: ParsedSelector = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.compound.len(), 2);
        assert!(back.compound[0].pseudo_hover);
        assert!(back.compound[0].pseudo_active);
        assert_eq!(back.compound[1].classes, vec!["child".to_string()]);
    }

    #[test]
    fn child_combinator_requires_direct_parent() {
        // nodes: 0=div.a (root), 1=div (普通), 2=span
        let mut nodes = vec![
            el("div", &["a"], None),
            el("div", &[], None),
            el("span", &[], None),
        ];
        nodes[1].parent = Some(ElementId(0));
        nodes[2].parent = Some(ElementId(1));
        nodes[0].children = vec![ElementId(1)];
        nodes[1].children = vec![ElementId(2)];
        let tree = ElementTree {
            roots: vec![ElementId(0)],
            nodes,
        };
        let rules = vec![
            Rule {
                selector_text: "div.a > span".into(),
                declarations: vec![],
            },
            Rule {
                selector_text: "div.a span".into(),
                declarations: vec![],
            },
        ];
        let matched = match_element(&tree.nodes[2], &tree, &rules);
        // 子代 `div.a > span` 不中（直接父是普通 div，非 div.a）；
        // 后代 `div.a span` 中（div.a 是祖先）。共命中 1 条。
        assert_eq!(
            matched.len(),
            1,
            "div.a > span 不应命中（直接父非 div.a），div.a span 应命中"
        );
        assert_eq!(matched[0].selector_text, "div.a span");
    }
}
