//! Scene 层：持久 Node 树（场景图）。
//!
//! 消费 `ElementTree` + `Vec<ResolvedStyle>`，构建一棵 `Node` 树。
//! layout 层后续往 `taffy_id`/`layout_rect` 写几何；render 层消费
//! `clip_rect`/`dirty_*`。本模块只管建树 + 初始脏标志。

#[cfg(feature = "parse")]
use crate::parse::dom::{ElementId, ElementTree};
use crate::style::resolved::{OverflowMode, ResolvedStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

/// 默认 `Container`（无数据变体），render 层测试构造 Node 用 `Default::default()`。
#[derive(Debug, Clone, Default)]
pub enum NodeKind {
    #[default]
    Container,
    Text { content: String },
    /// src 原样存（不加载），render 层映射到占位 tex_id。
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
    /// 打包期 resolve_styles 产物（不变，rematch 基线）。style 是运行时 rematch 覆写值。
    pub base_style: ResolvedStyle,
    /// 运行时 class 列表（建树时从 ElementData.classes 填；供动态规则 class 选择器匹配）。
    pub classes: Vec<String>,
    /// 运行时 id（建树时从 ElementData.id 填；供动态规则 id 选择器匹配）。
    pub id_attr: Option<String>,
    /// pointer-events:auto=true / none=false（解析时落，建树时从 style.touchable 填）。
    pub touchable: bool,
    /// 当前帧命中（运行时，每帧命中 diff 更新）。
    pub hovered: bool,
    /// 指针按下且命中（运行时状态机）。
    pub active: bool,
    /// 业务设（set_node_disabled），伪类源 + active/click 抑制。
    pub disabled: bool,
    /// opt-in 可拖拽（HTML `draggable="true"` 属性）。drag 状态机据此发起 drag。
    pub draggable: bool,
    /// HTML tabindex 属性值。None=不可聚焦；Some(-1)=仅编程聚焦；
    /// Some(0)=DOM 序可聚焦；Some(N>0)=显式序可聚焦。
    pub tabindex: Option<i32>,
    /// 当前是否聚焦（运行时，:focus 伪类源）。仅 focused_node 链上节点 true。
    pub focused: bool,
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
            base_style: ResolvedStyle::default(),
            classes: Vec::new(),
            id_attr: None,
            touchable: true,
            hovered: false,
            active: false,
            disabled: false,
            draggable: false,
            tabindex: None,
            focused: false,
        }
    }
}

/// 单节点动画 override（replace-override：Some 覆盖 ResolvedStyle 对应字段，None 退回 CSS）。
/// 全 None = 无动画。由 TweenManager.update 写，由 compute_world_transforms / build_render_nodes 读。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NodeAnim {
    pub opacity: Option<f32>,
    pub transform: Option<crate::transform::Affine2>, // 覆盖 style.transform.matrix
    pub bg_color: Option<[f32; 4]>,
    pub text_color: Option<[f32; 4]>,
}

impl NodeAnim {
    pub fn is_empty(&self) -> bool {
        self.opacity.is_none()
            && self.transform.is_none()
            && self.bg_color.is_none()
            && self.text_color.is_none()
    }
}

/// 每节点动画 override 表（index = NodeId.0）。运行时态，不进 pkg（同 world_transforms）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimTable(pub Vec<NodeAnim>);

impl AnimTable {
    pub fn get(&self, node: NodeId) -> Option<&NodeAnim> {
        self.0.get(node.0).filter(|a| !a.is_empty())
    }

    /// 增长到 n 并返回可变切片（update 调，确保 node_id 可索引）。
    pub fn ensure(&mut self, n: usize) -> &mut [NodeAnim] {
        if self.0.len() < n {
            self.0.resize(n, NodeAnim::default());
        }
        &mut self.0
    }

    /// 清该节点所有通道（回 CSS）。
    pub fn clear_node(&mut self, node: NodeId) {
        if let Some(a) = self.0.get_mut(node.0) {
            *a = NodeAnim::default();
        }
    }

    /// 清该节点某 prop 对应通道（Translate/Scale/Rotation 都映射到 transform 通道）。
    pub fn clear_prop(&mut self, node: NodeId, prop: crate::tween::TweenProp) {
        let a = match self.0.get_mut(node.0) {
            Some(a) => a,
            None => return,
        };
        use crate::tween::TweenProp;
        match prop {
            TweenProp::Opacity => a.opacity = None,
            TweenProp::Translate | TweenProp::Scale | TweenProp::Rotation => a.transform = None,
            TweenProp::BgColor => a.bg_color = None,
            TweenProp::TextColor => a.text_color = None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub roots: Vec<NodeId>,
    pub nodes: Vec<Node>,
    /// 运行时伪类重匹配规则表。默认空；包加载填，inline 路径空。
    pub dynamic_rules: crate::style::dynamic::DynamicRuleTable,
    /// 当前焦点节点（单一全局，照 fgui Stage.focus）。None=无焦点。
    pub focused_node: Option<NodeId>,
    /// 每节点累计世界矩阵（compute_world_transforms 填）。index = NodeId.0。运行时态，不进 pkg。
    pub world_transforms: Vec<crate::transform::Affine2>,
    /// 每节点动画 override（TweenManager.update 填）。index = NodeId.0。运行时态，不进 pkg。
    pub anim: AnimTable,
    /// 每节点滚动状态（refresh_content_sizes / scroll 物理填）。index = NodeId.0。运行时态，不进 pkg。
    pub scroll: crate::scroll::ScrollTable,
    /// 每节点 text 测量结果（layout solve 填，render 复用——消除双测量不一致）。
    /// index = NodeId.0，仅 Text 节点 Some。运行时态，不进 pkg。
    ///
    /// 根因：layout 闭包用 taffy 选定 max_width 测（短文本 intrinsic≈available → taffy 传 None
    /// 不换行；长文本 → Some(available) 换行），render 若用 rect.w（stretch 后的 available 整数宽）
    /// 重测，短文本因 intrinsic 亚像素超 available 误判换行。故 render 复用 layout 结果，不重测。
    pub text_layouts: Vec<Option<crate::text::layout::TextLayout>>,
}

impl Scene {
    /// 从扁平 entries（DFS 先序）建 Node 树。`NodeId = entries 下标`；
    /// `parent_idx` 指向 entries 下标，`None` = 根。
    /// clip_rect slot / dirty 标志按 style.overflow_x/y（非 Visible 即 clip）/ kind 派生。
    /// parse 路径（build_scene）与包加载路径（read_package）共用。
    pub fn build(entries: &[(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)]) -> Scene {
        let mut scene = Scene {
            roots: Vec::new(),
            nodes: Vec::new(),
            dynamic_rules: crate::style::dynamic::DynamicRuleTable::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        for (i, (parent_idx, kind, style, classes, id_attr, draggable, tabindex)) in entries.iter().enumerate() {
            scene.nodes.push(Node {
                id: NodeId(i),
                parent: parent_idx.map(NodeId),
                kind: kind.clone(),
                style: style.clone(),
                base_style: style.clone(),
                taffy_id: None,
                layout_rect: Rect::default(),
                clip_rect: if style.overflow_x != OverflowMode::Visible
                    || style.overflow_y != OverflowMode::Visible
                {
                    Some(Rect::default())
                } else {
                    None
                },
                children: Vec::new(),
                dirty_mesh: true,
                dirty_text: matches!(kind, NodeKind::Text { .. }),
                classes: classes.clone(),
                id_attr: id_attr.clone(),
                touchable: style.touchable,
                hovered: false,
                active: false,
                disabled: false,
                draggable: *draggable,
                tabindex: *tabindex,
                focused: false,
            });
        }
        // 接 children + roots（entries 先序 → 按 parent 出现序填）
        for i in 0..entries.len() {
            match entries[i].0 {
                Some(p) => scene.nodes[p].children.push(NodeId(i)),
                None => scene.roots.push(NodeId(i)),
            }
        }
        // text_layouts 随 nodes 长度对齐（None 占位，layout::solve 填实际 TextLayout）。
        scene.text_layouts = vec![None; scene.nodes.len()];
        scene
    }

    /// 按 CSS id 属性查节点（首个匹配）。无匹配 / 空 id → None。
    /// 供 FFI find_node_by_id：业务用 id 注册 listener / 设 disabled，替代硬编码 build 序 id
    /// （auto Text 子会偏移 build 序，硬编码不可靠）。
    pub fn find_by_id_attr(&self, id: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|n| n.id_attr.as_deref() == Some(id))
            .map(|n| n.id)
    }
}

/// 从 ElementTree + ResolvedStyle 构建 Node 树（gather 后调 `Scene::build`）。
///
/// `styles` 必须与 `tree.nodes` 同长且同序（由 `style::cascade::resolve_styles` 保证）。
#[cfg(feature = "parse")]
pub fn build_scene(tree: &ElementTree, styles: &[ResolvedStyle]) -> Scene {
    let mut entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = Vec::new();
    for root in &tree.roots {
        gather_rec(tree, styles, *root, None, &mut entries);
    }
    Scene::build(&entries)
}

#[cfg(feature = "parse")]
fn gather_rec(
    tree: &ElementTree,
    styles: &[ResolvedStyle],
    el_id: ElementId,
    parent_idx: Option<usize>,
    entries: &mut Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)>,
) -> usize {
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
            "parse 层白名单已挡围栏外 tag，scene 不应见到 <{}>；这是 parse/scene 契约破坏",
            el.tag
        ),
    };
    // draggable="true" → Node.draggable（HTML 原生属性）。
    // 非 "true" 一律 false（draggable="false"/缺省/任意值 → false，照 HTML truthy 语义简化）。
    let draggable = el.attrs.get("draggable").map(|v| v == "true").unwrap_or(false);
    // tabindex 属性 → Option<i32>。非数字 → None（照 DOM 容错：无效值忽略）。
    // 语义：None=不可聚焦；Some(-1)=仅编程；Some(0)=DOM 序；Some(N>0)=显式序。
    let tabindex = el.attrs.get("tabindex").and_then(|v| v.parse::<i32>().ok());
    let my_idx = entries.len();
    entries.push((parent_idx, kind.clone(), style.clone(), el.classes.clone(), el.id.clone(), draggable, tabindex));

    // Container/Button 的裸文本 → Text 子节点。文本子像无 class 的 <span>：
    // taffy_style 取 DEFAULT（由测量定尺寸），视觉/字体字段继承父值。
    // 不能直接克隆父 style——父若是 .h{height:30px} 会让文本子也高 30px，
    // 既不正确也压制了文本自然测量。
    if matches!(kind, NodeKind::Container | NodeKind::Button) {
        if let Some(text) = &el.text {
            let mut ts = ResolvedStyle::default();
            ts.color = style.color;
            ts.font_size = style.font_size;
            ts.font_family = style.font_family.clone();
            ts.font_weight = style.font_weight;
            ts.line_height = style.line_height;
            ts.letter_spacing = style.letter_spacing;
            ts.text_align = style.text_align;
            ts.white_space_nowrap = style.white_space_nowrap;
            entries.push((Some(my_idx), NodeKind::Text { content: text.clone() }, ts, Vec::new(), None, false, None));
        }
    }

    if !el.children.is_empty() {
        for c in &el.children {
            gather_rec(tree, styles, *c, Some(my_idx), entries);
        }
    }
    my_idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_has_runtime_state_fields_default() {
        let n = Node::default();
        assert!(n.touchable, "touchable 默认 true");
        assert!(!n.hovered);
        assert!(!n.active);
        assert!(!n.disabled);
        assert!(n.classes.is_empty());
        assert!(n.id_attr.is_none());
        // base_style 与 style 初始相同（Default）
        assert_eq!(n.base_style, n.style);
    }

    #[test]
    fn node_has_draggable_field_default_false() {
        let n = Node::default();
        assert!(!n.draggable, "draggable 默认 false");
    }

    #[test]
    fn scene_build_6tuple_sets_draggable() {
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), Vec::new(), None, true, None),
        ];
        let scene = Scene::build(&entries);
        assert!(!scene.nodes[0].draggable, "root draggable=false");
        assert!(scene.nodes[1].draggable, "btn draggable=true");
    }

    #[test]
    fn scene_default_has_empty_dynamic_rules() {
        let s = Scene {
            roots: vec![],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        assert!(
            s.dynamic_rules.rules.is_empty(),
            "Scene 默认 dynamic_rules 空"
        );
    }

    #[test]
    fn node_has_tabindex_focused_defaults() {
        let n = Node::default();
        assert_eq!(n.tabindex, None, "tabindex 默认 None（不可聚焦）");
        assert!(!n.focused, "focused 默认 false");
    }

    #[test]
    fn scene_default_focused_node_none() {
        let s = Scene {
            roots: vec![],
            nodes: vec![],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        assert_eq!(s.focused_node, None, "Scene 默认 focused_node=None");
    }

    #[test]
    fn scene_build_7tuple_sets_tabindex() {
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), Vec::new(), None, false, Some(0)),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), Vec::new(), None, false, Some(3)),
        ];
        let scene = Scene::build(&entries);
        assert_eq!(scene.nodes[0].tabindex, None, "root tabindex=None");
        assert_eq!(scene.nodes[1].tabindex, Some(0), "btn1 tabindex=Some(0)");
        assert_eq!(scene.nodes[2].tabindex, Some(3), "btn2 tabindex=Some(3)");
        assert!(!scene.nodes[0].focused, "focused 默认 false");
        assert_eq!(scene.focused_node, None, "build 后 focused_node=None");
    }

    #[test]
    fn scene_build_constructs_tree_without_parse() {
        // 手搓 entries：root Container + 一个 Text 子（parent=Some(0)）。
        // 不走 parse_html/build_scene——证明 Scene::build 独立于 parse（read_package 依赖此）。
        let root_style = ResolvedStyle::default();
        let text_style = ResolvedStyle::default();
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, root_style, Vec::new(), None, false, None),
            (Some(0), NodeKind::Text { content: "hi".into() }, text_style, Vec::new(), None, false, None),
        ];
        let scene = Scene::build(&entries);

        assert_eq!(scene.nodes.len(), 2);
        assert_eq!(scene.roots, vec![NodeId(0)], "根 = parent=None 的节点");
        let root = &scene.nodes[0];
        assert!(matches!(root.kind, NodeKind::Container));
        assert_eq!(root.children, vec![NodeId(1)], "Text 子挂 root");
        assert!(root.clip_rect.is_none(), "overflow Visible → 无 clip slot");
        assert!(!root.dirty_text, "Container dirty_text=false");
        let text = &scene.nodes[1];
        assert!(matches!(&text.kind, NodeKind::Text { content } if content == "hi"));
        assert_eq!(text.parent, Some(NodeId(0)));
        assert!(text.dirty_text, "Text 节点 dirty_text=true");

        // overflow Hidden → clip slot 派生
        let mut of = ResolvedStyle::default();
        of.overflow_x = OverflowMode::Hidden;
        of.overflow_y = OverflowMode::Hidden;
        let scene2 = Scene::build(&[(None, NodeKind::Container, of, Vec::new(), None, false, None)]);
        assert!(scene2.nodes[0].clip_rect.is_some(), "overflow Hidden → clip slot");
    }

    #[test]
    fn build_clip_rect_slot_for_scroll_auto_and_single_axis() {
        // overflow != Visible（任一轴）→ clip slot。覆盖 scroll/auto/单轴。
        for (x, y, desc) in [
            (OverflowMode::Scroll, OverflowMode::Scroll, "scroll 双轴"),
            (OverflowMode::Auto, OverflowMode::Auto, "auto 双轴"),
            (OverflowMode::Scroll, OverflowMode::Visible, "仅 x 轴 scroll"),
            (OverflowMode::Visible, OverflowMode::Auto, "仅 y 轴 auto"),
        ] {
            let mut s = ResolvedStyle::default();
            s.overflow_x = x;
            s.overflow_y = y;
            let sc = Scene::build(&[(None, NodeKind::Container, s, Vec::new(), None, false, None)]);
            assert!(sc.nodes[0].clip_rect.is_some(), "{} → clip slot", desc);
        }
        // 双轴 Visible → 无 clip slot（对照）
        let mut vis = ResolvedStyle::default();
        vis.overflow_x = OverflowMode::Visible;
        vis.overflow_y = OverflowMode::Visible;
        let sc = Scene::build(&[(None, NodeKind::Container, vis, Vec::new(), None, false, None)]);
        assert!(sc.nodes[0].clip_rect.is_none(), "双轴 Visible → 无 clip slot");
    }

    #[test]
    fn animtable_get_returns_none_for_empty_or_unset() {
        let mut t = AnimTable::default();
        t.ensure(3);
        // 全默认（None）→ get 返 None（is_empty 过滤）
        assert!(t.get(NodeId(0)).is_none());
        assert!(t.get(NodeId(5)).is_none(), "越界 → None");
    }

    #[test]
    fn animtable_clear_prop_transform_channel_shared() {
        let mut t = AnimTable::default();
        t.ensure(2);
        t.0[1].transform = Some(crate::transform::from_scale(2.0, 2.0));
        // Translate/Scale/Rotation 都清 transform 通道
        t.clear_prop(NodeId(1), crate::tween::TweenProp::Scale);
        assert!(t.0[1].transform.is_none(), "clear Scale → transform 通道 None");
    }

    #[test]
    fn nodeanim_is_empty_default_true() {
        assert!(NodeAnim::default().is_empty());
        assert!(!NodeAnim { opacity: Some(0.5), ..Default::default() }.is_empty());
    }
}

// 依赖 parse 的测（build_scene via parse）——gate
#[cfg(all(test, feature = "parse"))]
mod parse_tests {
    use super::*;
    use crate::parse::{css::parse_css, dom::parse_html};
    use crate::style::cascade::resolve_styles;

    #[test]
    fn build_scene_fills_classes_and_id() {
        let html = r#"<div class="a b" id="x"><span class="c">hi</span></div>"#;
        let css = "";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let root = &scene.nodes[scene.roots[0].0];
        assert_eq!(root.classes, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(root.id_attr.as_deref(), Some("x"));
        let span = &scene.nodes[root.children[0].0];
        assert_eq!(span.classes, vec!["c".to_string()]);
    }

    #[test]
    fn find_by_id_attr_returns_node_and_none() {
        // 手搓 Scene（不走 parse）：root(id="root") + btn(id="btn") + Text 子(无 id)。
        // 验：精确匹配返 NodeId；无匹配/空 id → None。
        let entries = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), vec![], Some("root".to_string()), false, None),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), vec![], Some("btn".to_string()), false, None),
            (Some(1), NodeKind::Text { content: "x".into() }, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let scene = Scene::build(&entries);
        assert_eq!(scene.find_by_id_attr("btn"), Some(NodeId(1)), "find btn → node 1");
        assert_eq!(scene.find_by_id_attr("root"), Some(NodeId(0)), "find root → node 0");
        assert_eq!(scene.find_by_id_attr("missing"), None, "无匹配 → None");
        assert_eq!(scene.find_by_id_attr(""), None, "空 id → None");
    }

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
        // div 的裸文本 → Text 子节点（文本是 flex item，参与布局）。
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
        // Text 子节点应像无 class 的 <span>——继承父 color/font，
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

    #[test]
    fn draggable_attr_true_sets_node_draggable() {
        let html = r#"<div><button draggable="true">OK</button></div>"#;
        let css = "";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let root = &scene.nodes[scene.roots[0].0];
        // btn 是 root 的子（root 的 Text 子"OK"另算——button 裸文本→Text 子）。
        // 找 button kind 的子：
        let btn = scene.nodes.iter().find(|n| matches!(n.kind, NodeKind::Button)).expect("btn");
        assert!(btn.draggable, "draggable=\"true\" → Node.draggable=true");
        assert!(!root.draggable, "root 无 draggable 属性 → false");
    }

    #[test]
    fn draggable_attr_absent_or_false_is_false() {
        let html = r#"<div draggable="false"><button draggable="yes">x</button></div>"#;
        let css = "";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let root = &scene.nodes[scene.roots[0].0];
        assert!(!root.draggable, "draggable=\"false\" → false");
        let btn = scene.nodes.iter().find(|n| matches!(n.kind, NodeKind::Button)).expect("btn");
        assert!(!btn.draggable, "draggable=\"yes\"（非 true）→ false（truthy 仅认 true）");
    }

    #[test]
    fn tabindex_attr_parsed() {
        let html = r#"<div><button tabindex="0">a</button><button tabindex="3">b</button><button tabindex="-1">c</button><button tabindex="abc">d</button><button>e</button></div>"#;
        let css = "";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let scene = build_scene(&tree, &styles);
        let btns: Vec<_> = scene.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Button)).collect();
        assert_eq!(btns.len(), 5);
        assert_eq!(btns[0].tabindex, Some(0), "tabindex=\"0\" → Some(0)");
        assert_eq!(btns[1].tabindex, Some(3), "tabindex=\"3\" → Some(3)");
        assert_eq!(btns[2].tabindex, Some(-1), "tabindex=\"-1\" → Some(-1)");
        assert_eq!(btns[3].tabindex, None, "tabindex=\"abc\"（非数字）→ None");
        assert_eq!(btns[4].tabindex, None, "无 tabindex 属性 → None");
    }
}
