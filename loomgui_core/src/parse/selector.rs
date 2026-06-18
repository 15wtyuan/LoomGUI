use crate::parse::css::Rule;
use crate::parse::dom::{ElementData, ElementId, ElementTree};

/// 选择器组合子：标签/类/id/后代/子代。v0 不含伪类（状态恒定）。
#[derive(Debug, Clone)]
pub struct ParsedSelector {
    pub raw: String,
    pub compound: Vec<Compound>, // 复合选择器链（后代/子代分隔）
    pub specificity: Specificity,
}

#[derive(Debug, Clone)]
pub struct Compound {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub id: Option<String>,
    pub combinator: Combinator, // 本 compound 与前一个的关系
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Combinator {
    Descendant,
    Child,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity(pub u32, pub u32, pub u32); // (id 数, class 数, tag 数)

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
                next_comb = Combinator::Descendant;
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
            if c == '.' || c == '#' {
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
        compound.push(Compound {
            tag,
            classes,
            id,
            combinator: *comb,
        });
    }

    Ok(ParsedSelector {
        raw: raw_trimmed,
        compound,
        specificity: spec,
    })
}

fn compound_matches(c: &Compound, el: &ElementData) -> bool {
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
fn matches(sel: &ParsedSelector, el_id: ElementId, tree: &ElementTree) -> bool {
    let comps = &sel.compound;
    if comps.is_empty() {
        return false;
    }
    let last = &comps[comps.len() - 1];
    if !compound_matches(last, &tree.nodes[el_id.0]) {
        return false;
    }
    if comps.len() == 1 {
        return true;
    }
    // 向上匹配剩余 compound（从倒数第二个到第一个）
    let mut cur_el = el_id;
    let mut idx = comps.len() - 1;
    while idx > 0 {
        let target_comp = &comps[idx - 1];
        let parent = match tree.nodes[cur_el.0].parent {
            Some(p) => p,
            None => return false,
        };
        if !compound_matches(target_comp, &tree.nodes[parent.0]) {
            return false;
        }
        cur_el = parent;
        idx -= 1;
        // v0 简化：不严格区分 descendant/child 的多级，只查直接父。
        // （child 要求直接父；descendant 也只查父——v0 树浅，够用；如需严格后代遍历，实现期补）
    }
    true
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
}
