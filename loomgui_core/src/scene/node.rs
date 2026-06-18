//! Scene 层：持久 Node 树（场景图）。
//!
//! 消费 `ElementTree` + `Vec<ResolvedStyle>`，构建一棵 `Node` 树。
//! layout 层后续往 `taffy_id`/`layout_rect` 写几何；render 层消费
//! `clip_rect`/`dirty_*`。本模块只管建树 + 初始脏标志。

use crate::parse::dom::{ElementId, ElementTree};
use crate::style::resolved::ResolvedStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

/// 默认 `Container`（无数据变体），render 层测试构造 Node 用 `Default::default()`。
#[derive(Debug, Clone, Default)]
pub enum NodeKind {
    #[default]
    Container,
    Text { content: String },
    /// v0：src 原样存（不加载），render 层映射到占位 tex_id。
    /// src 取自元素的 `src` 属性（`<img src="...">`），不是文本内容。
    Image { src: String },
    Button,
}

#[derive(Debug, Clone, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub kind: NodeKind,
    pub style: ResolvedStyle,
    /// taffy 节点 id（layout 层建立映射后填）。
    pub taffy_id: Option<taffy::NodeId>,
    /// taffy solve 后写（父坐标系）。
    pub layout_rect: Rect,
    /// overflow:hidden 时为本节点 border 框（Some 占位，值由 layout/render 填）。
    pub clip_rect: Option<Rect>,
    /// 仅 Container/Button 有（Text/Image 为叶子）。
    pub children: Vec<NodeId>,
    pub dirty_mesh: bool,
    pub dirty_text: bool,
}

impl Default for Node {
    /// render 层 batch 测试构造占位 Node 用。
    /// id/parent/children 取空值，kind=Container（NodeKind::default），
    /// style 取 ResolvedStyle::default，layout_rect/clip_rect 取空。
    fn default() -> Self {
        Node {
            id: NodeId(0),
            parent: None,
            kind: NodeKind::default(),
            style: ResolvedStyle::default(),
            taffy_id: None,
            layout_rect: Rect::default(),
            clip_rect: None,
            children: Vec::new(),
            dirty_mesh: true,
            dirty_text: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub roots: Vec<NodeId>,
    pub nodes: Vec<Node>,
}

/// 从 ElementTree + ResolvedStyle 构建 Node 树。
///
/// `styles` 必须与 `tree.nodes` 同长且同序（由 `style::cascade::resolve_styles` 保证）。
pub fn build_scene(tree: &ElementTree, styles: &[ResolvedStyle]) -> Scene {
    let mut scene = Scene {
        roots: Vec::new(),
        nodes: Vec::new(),
    };
    for root in &tree.roots {
        let id = build_rec(tree, styles, *root, None, &mut scene);
        scene.roots.push(id);
    }
    scene
}

fn build_rec(
    tree: &ElementTree,
    styles: &[ResolvedStyle],
    el_id: ElementId,
    parent: Option<NodeId>,
    scene: &mut Scene,
) -> NodeId {
    let el = &tree.nodes[el_id.0];
    let style = &styles[el_id.0];
    // img 的 src 从属性取（`<img src="...">`），不是元素文本。
    // 属性缺失时空字符串降级（render 层负责报缺图占位）。
    // 未识别 tag 一律降级为 Text（v0 不报错；上层可选地校验白名单）。
    let kind = match el.tag.as_str() {
        "div" | "l-container" => NodeKind::Container,
        "button" => NodeKind::Button,
        "img" => NodeKind::Image {
            src: el.attrs.get("src").cloned().unwrap_or_default(),
        },
        _ => NodeKind::Text {
            content: el.text.clone().unwrap_or_default(),
        },
    };
    let has_children = !el.children.is_empty();
    let nid = NodeId(scene.nodes.len());
    scene.nodes.push(Node {
        id: nid,
        parent,
        kind: kind.clone(),
        style: style.clone(),
        taffy_id: None,
        layout_rect: Rect::default(),
        clip_rect: if style.overflow_hidden {
            Some(Rect::default())
        } else {
            None
        },
        children: Vec::new(),
        dirty_mesh: true,
        dirty_text: matches!(kind, NodeKind::Text { .. }),
    });
    if has_children {
        let mut kids = Vec::new();
        for c in &el.children {
            let cid = build_rec(tree, styles, *c, Some(nid), scene);
            kids.push(cid);
        }
        scene.nodes[nid.0].children = kids;
    }
    nid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{css::parse_css, dom::parse_html};
    use crate::style::cascade::resolve_styles;

    #[test]
    fn builds_div_button_text_image() {
        // img 用属性 src（不是文本）；其它元素覆盖四种 NodeKind。
        let html = r#"<div class="root"><button>OK</button><span>hi</span><img src="logo.png"></div>"#;
        let css = ".root { width: 200px; }";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let root = &scene.nodes[scene.roots[0].0];
        assert!(matches!(root.kind, NodeKind::Container));
        assert_eq!(root.children.len(), 3);
        assert!(matches!(
            scene.nodes[root.children[0].0].kind,
            NodeKind::Button
        ));
        let text = &scene.nodes[root.children[1].0];
        match &text.kind {
            NodeKind::Text { content } => assert_eq!(content, "hi"),
            _ => panic!("expected Text"),
        }
        match &scene.nodes[root.children[2].0].kind {
            NodeKind::Image { src } => assert_eq!(src, "logo.png"),
            _ => panic!("expected Image"),
        }
    }

    #[test]
    fn overflow_hidden_sets_clip_rect_slot() {
        let html = r#"<div></div>"#;
        let css = "div { overflow: hidden; }";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        assert!(scene.nodes[0].clip_rect.is_some());
    }

    #[test]
    fn image_without_src_falls_back_to_empty() {
        // 缺 src 属性不 panic，降级空串（render 层报缺图）。
        let html = r#"<div><img alt="no src"></div>"#;
        let css = "";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let root = &scene.nodes[scene.roots[0].0];
        match &scene.nodes[root.children[0].0].kind {
            NodeKind::Image { src } => assert_eq!(src, ""),
            _ => panic!("expected Image"),
        }
    }

    #[test]
    fn text_node_marks_dirty_text_and_clean_leaves_unset() {
        // Text 节点 dirty_text=true；Container dirty_text=false；全部 dirty_mesh=true。
        let html = r#"<div><span>hi</span></div>"#;
        let css = "";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let root = &scene.nodes[scene.roots[0].0];
        assert!(root.dirty_mesh);
        assert!(!root.dirty_text); // Container 不脏文本
        let text = &scene.nodes[root.children[0].0];
        assert!(text.dirty_mesh);
        assert!(text.dirty_text); // Text 节点脏文本
    }
}
