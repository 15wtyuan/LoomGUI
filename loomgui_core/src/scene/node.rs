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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
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
    // parse 层已保证 tag 在围栏白名单内（div/span/img/button/l-container），
    // 故此处显式 match 无 fallback；若来未识别 tag 是 parse/白名单的 bug。
    // img 的 src 从属性取（`<img src="...">`），不是元素文本；
    // span 的文本是其自身 content（Text 叶子，无子节点）。
    let kind = match el.tag.as_str() {
        "div" | "l-container" => NodeKind::Container,
        "button" => NodeKind::Button,
        "img" => NodeKind::Image {
            src: el.attrs.get("src").cloned().unwrap_or_default(),
        },
        "span" => NodeKind::Text {
            content: el.text.clone().unwrap_or_default(),
        },
        _ => unreachable!(
            "parse 层白名单已挡围栏外 tag，scene 不应见到 <{}>；\
             这是 parse/scene 契约破坏",
            el.tag
        ),
    };
    let has_element_children = !el.children.is_empty();
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

    // §4.2：Container/Button 的裸文本 → Text 子节点（文本是 flex item，参与布局）。
    // parse 已保证 has_text 时无元素子（行内混排编译期报错），故此处只需判 el.text。
    // 文本子放 children 头（混排已挡，只有纯文本无元素子场景，顺序无所谓）。
    let mut kids: Vec<NodeId> = Vec::new();
    if matches!(kind, NodeKind::Container | NodeKind::Button) {
        if let Some(text) = &el.text {
            let text_nid = build_text_child(scene, text, Some(nid));
            kids.push(text_nid);
        }
    }

    if has_element_children {
        for c in &el.children {
            let cid = build_rec(tree, styles, *c, Some(nid), scene);
            kids.push(cid);
        }
    }
    scene.nodes[nid.0].children = kids;
    nid
}

/// 构造一个 Text 叶子 Node（Container/Button 的裸文本子节点）。
/// 注意：span 的文本是其自身 content（在 build_rec 里直接进 NodeKind::Text），
/// 不经此函数；本函数只服务 Container/Button 的裸文本→Text 子节点转换。
fn build_text_child(scene: &mut Scene, text: &str, parent: Option<NodeId>) -> NodeId {
    let nid = NodeId(scene.nodes.len());
    // Text 子节点（对应 HTML 里无独立元素）应像无 class 的 <span> 一样：
    // taffy_style 取 DEFAULT（无固定 size，由文本测量决定尺寸，正常参与 flex），
    // 视觉/字体字段继承父值（cascade 里 <span> 也是这么拿 color/font-* 的）。
    // 不能直接克隆父 style——父若是 .h{height:30px} 会让文本子也高 30px，
    // 既不正确也压制了文本自然测量。
    let mut style = ResolvedStyle::default();
    if let Some(p) = parent {
        let ps = &scene.nodes[p.0].style;
        style.color = ps.color;
        style.font_size = ps.font_size;
        style.font_family = ps.font_family.clone();
        style.font_weight = ps.font_weight;
        style.line_height = ps.line_height;
        style.letter_spacing = ps.letter_spacing;
        style.text_align = ps.text_align;
        style.white_space_nowrap = ps.white_space_nowrap;
    }
    scene.nodes.push(Node {
        id: nid,
        parent,
        kind: NodeKind::Text {
            content: text.to_string(),
        },
        style,
        taffy_id: None,
        layout_rect: Rect::default(),
        clip_rect: None,
        children: Vec::new(),
        dirty_mesh: true,
        dirty_text: true,
    });
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

    #[test]
    fn div_raw_text_becomes_text_child() {
        // §4.2：div 的裸文本 → Text 子节点（文本是 flex item，参与布局）。
        // 匹配 AI 的 HTML 先验：<div>标题</div> 里的"标题"应可见、参与 flex 排列。
        let html = r#"<div>标题</div>"#;
        let css = "";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let root = &scene.nodes[scene.roots[0].0];
        assert!(matches!(root.kind, NodeKind::Container));
        assert_eq!(root.children.len(), 1, "裸文本应产 1 个 Text 子节点");
        let child = &scene.nodes[root.children[0].0];
        match &child.kind {
            NodeKind::Text { content } => assert_eq!(content, "标题"),
            other => panic!("expected Text child, got {:?}", other),
        }
        // parent 指向 Container
        assert_eq!(child.parent, Some(scene.roots[0]));
    }

    #[test]
    fn button_raw_text_becomes_text_child() {
        // button 同理：裸文本 → Text 子节点
        let html = r#"<button>确定</button>"#;
        let css = "";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let btn = &scene.nodes[scene.roots[0].0];
        assert!(matches!(btn.kind, NodeKind::Button));
        assert_eq!(btn.children.len(), 1);
        match &scene.nodes[btn.children[0].0].kind {
            NodeKind::Text { content } => assert_eq!(content, "确定"),
            _ => panic!("expected Text child"),
        }
    }

    #[test]
    fn text_child_inherits_parent_text_fields_resets_size() {
        // §4.2 + §5.2.3：Text 子节点应像无 class 的 <span>——继承父 color/font，
        // 但 taffy_style 取 DEFAULT（无固定 size，由测量决定）。
        // 父 .h{height:30px} 不应让文本子也高 30px。
        let html = r#"<div class="h">txt</div>"#;
        let css = r#".h { height: 30px; color: #ff0000; font-size: 20px; }"#;
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let root = &scene.nodes[scene.roots[0].0];
        let child = &scene.nodes[root.children[0].0];
        // 继承
        assert_eq!(child.style.color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(child.style.font_size, 20.0);
        // size 不继承：父 height=Length(30)，子 height 应是 Auto（由文本测量决定）
        use taffy::style::Dimension;
        assert!(
            matches!(child.style.taffy_style.size.height, Dimension::Auto),
            "text child height should be Auto (measured), not inherited parent's 30px"
        );
    }
}
