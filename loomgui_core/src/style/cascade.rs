use crate::parse::css::{parse_inline_style, Rule, StyleSheet};
use crate::parse::dom::{ElementId, ElementTree};
use crate::parse::selector::{match_element, parse_selector, Specificity};
use crate::style::mapping::apply_decl;
use crate::style::resolved::ResolvedStyle;

/// 给整棵树算每元素的 ResolvedStyle（继承在 resolve 期展开）。
/// 索引与 ElementTree.nodes 一一对应。
pub fn resolve_styles(tree: &ElementTree, sheet: &StyleSheet) -> Vec<ResolvedStyle> {
    let mut out: Vec<ResolvedStyle> = (0..tree.nodes.len()).map(|_| ResolvedStyle::default()).collect();
    // 根元素用默认；自顶向下，每元素从父继承白名单后再叠自己的命中规则。
    fn resolve_rec(
        tree: &ElementTree,
        sheet: &StyleSheet,
        id: ElementId,
        parent: Option<&ResolvedStyle>,
        out: &mut [ResolvedStyle],
    ) {
        let mut style = ResolvedStyle::default();
        if let Some(p) = parent {
            // 继承白名单字段
            style.color = p.color;
            style.font_size = p.font_size;
            style.font_family = p.font_family.clone();
            style.font_weight = p.font_weight;
            style.line_height = p.line_height;
            style.letter_spacing = p.letter_spacing;
            style.text_align = p.text_align;
            style.white_space_nowrap = p.white_space_nowrap;
        }

        let el = &tree.nodes[id.0];
        let rules = match_element(el, tree, &sheet.rules);
        // CSS cascade：低 specificity 先 apply，高 specificity 后 apply（覆盖）；
        // 同 specificity 按源码顺序（后写的覆盖）。match_element 返回的是
        // (specificity DESC, source ASC)，这里按 specificity 升序稳定重排，
        // 得到 (specificity ASC, source ASC) —— 高 specificity 最后 apply 胜出，
        // 同 specificity 时源码靠后的最后 apply 胜出。
        let mut rules: Vec<&Rule> = rules;
        rules.sort_by_key(|r| specificity_of(r));
        for rule in rules {
            for decl in &rule.declarations {
                apply_decl(&mut style, &decl.prop, &decl.value);
            }
        }
        // inline style（specificity 最高，> id > class）：sheet rules 之后 apply，最后胜出。
        // 色块 style="background-color:..." / flex 容器 style="flex-direction:..." 等靠此生效。
        if let Some(style_str) = el.attrs.get("style") {
            for decl in parse_inline_style(style_str) {
                apply_decl(&mut style, &decl.prop, &decl.value);
            }
        }
        out[id.0] = style;
        // 借用检查：resolve_rec 同时要 parent=&ResolvedStyle 和 &mut out，
        // 两者在 out 上冲突。把 parent 克隆出来脱离 out 的借用即可。
        // （仅克隆一个 ResolvedStyle/子节点，font_family 是唯一堆分配，开销可接受。）
        let owned_style = out[id.0].clone();
        for child in &el.children {
            resolve_rec(tree, sheet, *child, Some(&owned_style), out);
        }
    }
    for root in &tree.roots {
        resolve_rec(tree, sheet, *root, None, &mut out);
    }
    out
}

/// 取一条 rule 选择器的 specificity。解析失败回退到 (0,0,0) 最低（安全降级）。
fn specificity_of(rule: &Rule) -> Specificity {
    parse_selector(&rule.selector_text)
        .map(|s| s.specificity)
        .unwrap_or(Specificity(0, 0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{css::parse_css, dom::parse_html};

    #[test]
    fn inheritance_propagates_color() {
        // 用 <style> 块测继承（class 规则驱动）。
        let html2 = r#"<div class="root"><span class="child">hi</span></div>"#;
        let css = r#".root { color: #ff0000; font-size: 20px; } .child { width: 50px; }"#;
        let tree = parse_html(html2).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        // root（div.root）应是红 20px
        let root_id = tree.roots[0];
        assert_eq!(styles[root_id.0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(styles[root_id.0].font_size, 20.0);
        // child 继承 color/font-size，自身 width=50px
        let child_id = tree.nodes[root_id.0].children[0];
        assert_eq!(styles[child_id.0].color, [1.0, 0.0, 0.0, 1.0]); // 继承
        assert_eq!(styles[child_id.0].font_size, 20.0); // 继承
    }

    #[test]
    fn specificity_order() {
        let html = r#"<div id="x" class="a"></div>"#;
        let css = r#"div { color: #000000; } .a { color: #ff0000; } #x { color: #00ff00; }"#;
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let id = tree.roots[0];
        // #x 胜（id specificity 最高）
        assert_eq!(styles[id.0].color, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn pseudo_class_rules_excluded_from_base_cascade() {
        // 伪类规则（:hover/:active/:focus）不该进 base_style——运行时 rematch 按状态动态应用。
        // compound_matches 须跳过含伪类的 compound，否则 .btn:focus 紫 / .btn:hover 蓝 / .btn:active 红
        // 污染 .btn base（specificity 同级 (0,2,0)，源序最后者胜 → base 变最后一条伪类色）。
        let html = r#"<div class="root"><button class="btn">OK</button></div>"#;
        let css = r#".btn { background-color: #888888; } .btn:hover { background-color: #2266ff; } .btn:active { background-color: #ff4444; } .btn:focus { background-color: #cc44cc; }"#;
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let btn_id = tree.nodes[tree.roots[0].0].children[0];
        let bg = styles[btn_id.0].background_color.expect(".btn 有 bg");
        // base 该是 .btn 灰 #888888，不是任一伪类色
        assert_eq!(
            bg,
            [136.0 / 255.0, 136.0 / 255.0, 136.0 / 255.0, 1.0],
            "base 该是 .btn 灰，不该被伪类规则污染（修复前会被 :focus 紫污染）"
        );
    }

    #[test]
    fn inline_style_applies_background_color() {
        // 色块：<div class="sw" style="background-color:#1a1d2e"> —— 颜色靠 inline
        // （.sw class 只给 width/height；不解析 inline → bg 缺失 → 透明看不见）。
        let html = r#"<div class="root"><div class="sw" style="background-color:#1a1d2e"></div></div>"#;
        let css = r#".sw { width: 60px; height: 60px; }"#;
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let sw_id = tree.nodes[tree.roots[0].0].children[0];
        let bg = styles[sw_id.0].background_color.expect(".sw inline bg");
        assert_eq!(bg, [0x1a as f32 / 255.0, 0x1d as f32 / 255.0, 0x2e as f32 / 255.0, 1.0]);
    }

    #[test]
    fn inline_style_overrides_class_rule() {
        // CSS specificity：inline > id > class。inline 必须最后 apply 胜出。
        let html = r#"<div class="root"><div class="a" style="color:#00ff00"></div></div>"#;
        let css = r#".a { color: #ff0000; }"#;
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let id = tree.nodes[tree.roots[0].0].children[0];
        assert_eq!(styles[id.0].color, [0.0, 1.0, 0.0, 1.0], "inline 绿胜 class 红");
    }
}
