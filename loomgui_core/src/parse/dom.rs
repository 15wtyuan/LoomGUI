use scraper::{Html, Selector as ScraperSelector};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ElementId(pub usize);

#[derive(Debug, Clone)]
pub struct ElementData {
    pub tag: String,
    pub classes: Vec<String>,
    pub id: Option<String>,
    pub text: Option<String>,
    pub children: Vec<ElementId>,
    pub parent: Option<ElementId>,
}

#[derive(Debug, Clone)]
pub struct ElementTree {
    pub roots: Vec<ElementId>,
    pub nodes: Vec<ElementData>,
}

/// 相邻文本节点合并为一个 Text 叶子；行内混排（元素内"文本+元素+文本"）报错。
pub fn parse_html(html: &str) -> Result<ElementTree, String> {
    let document = Html::parse_document(html);
    // 取根（body 子节点，忽略 html/head/body 包裹）
    let body_sel = ScraperSelector::parse("body").unwrap();
    let body = document.select(&body_sel).next().ok_or("no body")?;

    let mut tree = ElementTree {
        roots: Vec::new(),
        nodes: Vec::new(),
    };
    for child_node in body.children() {
        // 只关心元素子节点；文本/注释等忽略
        if child_node.value().is_element() {
            if let Some(child_el) = scraper::ElementRef::wrap(child_node) {
                let id = build_element(&mut tree, child_el, None)?;
                tree.roots.push(id);
            }
        }
    }
    Ok(tree)
}

fn build_element(
    tree: &mut ElementTree,
    el_node: scraper::ElementRef,
    parent: Option<ElementId>,
) -> Result<ElementId, String> {
    let el_val = el_node.value();
    let tag = el_val.name().to_string();
    let id_attr = el_val.attr("id").map(|s| s.to_string());
    let classes = el_val
        .attr("class")
        .map(|s| s.split_whitespace().map(|w| w.to_string()).collect())
        .unwrap_or_default();

    // 收集直接文本子节点；若同时有文本子节点和元素子节点 → 行内混排报错
    let mut text_parts: Vec<String> = Vec::new();
    let mut element_children: Vec<scraper::ElementRef> = Vec::new();
    for child in el_node.children() {
        match child.value() {
            scraper::node::Node::Text(t) => {
                let s = t.text.trim();
                if !s.is_empty() {
                    text_parts.push(s.to_string());
                }
            }
            scraper::node::Node::Element(_) => {
                if let Some(eref) = scraper::ElementRef::wrap(child) {
                    element_children.push(eref);
                }
            }
            _ => {}
        }
    }

    let has_text = !text_parts.is_empty();
    let has_elements = !element_children.is_empty();
    // 叶子文本元素（span/裸文本/button 文本）允许 text 无 element 子；
    // 容器（div 等）允许 element 子无 text；混排报错。
    if has_text && has_elements {
        return Err(format!(
            "行内混排不支持（div 只装 flex item）：元素 <{}> 同时含文本与子元素，改用单个文本或 l-rich",
            tag
        ));
    }

    let text = if has_text {
        Some(text_parts.join(" "))
    } else {
        None
    };

    let idx = tree.nodes.len();
    tree.nodes.push(ElementData {
        tag,
        classes,
        id: id_attr,
        text,
        children: Vec::new(),
        parent,
    });
    let my_id = ElementId(idx);

    let mut children_ids = Vec::new();
    for child_el in element_children {
        let cid = build_element(tree, child_el, Some(my_id))?;
        children_ids.push(cid);
    }
    tree.nodes[idx].children = children_ids;
    Ok(my_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_div_with_text() {
        let tree = parse_html(r#"<div class="panel">hello</div>"#).unwrap();
        assert_eq!(tree.roots.len(), 1);
        let root = &tree.nodes[tree.roots[0].0];
        assert_eq!(root.tag, "div");
        assert_eq!(root.classes, vec!["panel"]);
        assert_eq!(root.text.as_deref(), Some("hello"));
    }

    #[test]
    fn merges_adjacent_text_into_one_leaf() {
        let tree = parse_html(r#"<div>foo bar baz</div>"#).unwrap();
        let root = &tree.nodes[tree.roots[0].0];
        assert_eq!(root.text.as_deref(), Some("foo bar baz"));
        assert!(root.children.is_empty());
    }

    #[test]
    fn rejects_inline_mix_of_text_and_element() {
        // div 只装 flex item；文本+元素混排编译期报错
        let result = parse_html(r#"<div>hello <img src="a.png"> world</div>"#);
        assert!(result.is_err());
    }

    #[test]
    fn nested_containers() {
        let tree = parse_html(r#"<div><div><span>x</span></div></div>"#).unwrap();
        let outer = &tree.nodes[tree.roots[0].0];
        assert_eq!(outer.children.len(), 1);
        let inner = &tree.nodes[outer.children[0].0];
        assert_eq!(inner.children.len(), 1);
        let span = &tree.nodes[inner.children[0].0];
        assert_eq!(span.tag, "span");
    }
}
