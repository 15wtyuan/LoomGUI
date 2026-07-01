//! Scene 层：持久 Node 树（场景图）。
//!
//! 消费 `ElementTree` + `Vec<ResolvedStyle>`，构建一棵 `Node` 树。
//! layout 层后续往 `taffy_id`/`layout_rect` 写几何；render 层消费
//! `clip_rect`/`dirty_*`。本模块只管建树 + 初始脏标志。

#[cfg(feature = "parse")]
use crate::parse::dom::{ElementId, ElementTree};
use crate::style::resolved::{OverflowMode, ResolvedStyle};
use slotmap::{DefaultKey, Key, KeyData, SlotMap};

/// 不透明节点句柄。对外 u32（FFI/C# 透明），内部 = 高 20 bit index + 低 12 bit generation。
/// sentinel 0xFFFF_FFFF = INVALID。index 用于并行数组（anim/scroll/world_transforms）索引，
/// gen 由 slotmap 校验悬空。详见 v1.3+ 动态树 spec §3。
///
/// **与 slotmap 的衔接**（spec §3.2 实现期校准结果）：
/// slotmap 1.1.1 的 `new_key_type!` 生成的 Key 内部是 `KeyData { idx: u32, version: NonZeroU32 }`
/// （两字段均私有，仅 `as_ffi()/from_ffi()` 公开），其完整编码是 64 bit，**无法无损装入 u32**。
/// 而 FFI/C#/FrameBlob/`.pkg.bin` 全程硬约定 `node_id: u32` + sentinel `0xFFFF_FFFF`（spec §3.3、§7）。
/// 故不采用 `new_key_type!` 重定义 NodeId，而是保留 `NodeId(pub u32)`（应用层句柄），scene.nodes 用
/// `SlotMap<DefaultKey, Node>`，由 `Scene::key_for(NodeId)` 经 `KeyData::from_ffi` 桥接到 DefaultKey。
///
/// 位宽 20/12：index 20 bit（~100 万节点上限）+ generation 12 bit（4096 代，slotmap version ≤ 4095
/// 时无损；超过时 `key_for` 重构的 KeyData version 截断 → slotmap.get 安全返 None，符合 spec "4096 代足够"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);

impl NodeId {
    /// 无效句柄 sentinel（同 v1 FFI None/0 约定）。
    pub const INVALID: NodeId = NodeId(0xFFFF_FFFF);
    pub fn is_valid(self) -> bool {
        self.0 != 0xFFFF_FFFF
    }
    /// slotmap 槽位号（高 20 bit）。并行数组按此索引。
    pub fn index(self) -> usize {
        (self.0 >> 12) as usize
    }
    /// generation（低 12 bit）。slotmap 内部校验用。
    pub fn gen(self) -> u16 {
        (self.0 & 0xFFF) as u16
    }

    /// 从 slotmap DefaultKey 构造 NodeId（insert 后回填 Node.id / roots 用）。
    /// 编码：index = key.idx（slotmap 槽位号，1..=capacity），gen = key.version 低 12 bit。
    pub fn from_key(k: DefaultKey) -> NodeId {
        let ffi = k.data().as_ffi();
        let idx = (ffi & 0xFFFF_FFFF) as u32;
        let version = (ffi >> 32) as u32;
        NodeId((idx << 12) | (version & 0xFFF))
    }

    /// 重构 slotmap DefaultKey（Scene::get/get_mut 经此桥接）。
    /// slotmap KeyData::from_ffi 强制 version 奇数（与 slotmap 内部一致）。
    pub fn to_key(self) -> DefaultKey {
        let idx = (self.0 >> 12) as u64;
        let version = (self.0 & 0xFFF) as u64;
        DefaultKey::from(KeyData::from_ffi((version << 32) | idx))
    }
}

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

/// 每节点动画 override 表（HashMap<NodeId, NodeAnim>）。运行时态，不进 pkg（同 world_transforms）。
///
/// **T3 校准**：brief 原定 `SecondaryMap<NodeId, NodeAnim>`，但 slotmap 1.1 的 `Key` 是
/// `unsafe` trait（依赖 `KeyData` 内部不变量，slotmap 强烈建议用 `new_key_type!` 而非手 impl）；
/// 且 `KeyData` 内部是 `idx:u32 + version:NonZeroU32`（64 bit），与 NodeId 的 32 bit 应用句柄
/// 布局不匹配——手 `unsafe impl Key` 要把 20/12 编码强行映射到 32/32，语义错位比 HashMap 危险。
/// 故 `SecondaryMap<NodeId, _>` 不可行。T2 的 DefaultKey 桥接使 anim/scroll 的访问句柄是 NodeId，
/// 若用 `SecondaryMap<DefaultKey, _>` 则每次访问要 `NodeId::to_key()` 转换，且 SecondaryMap 不
/// 自动跟踪主 SlotMap 的删除（删节点须手动 remove，否则残留）。改用 `HashMap<NodeId, NodeAnim>`：
/// NodeId 已 derive Hash+Eq（T1），零 trait 限制、零转换、悬空安全（删节点联动 remove，否则
/// 残留条目但 get 用 live NodeId 查不到）。查询 O(1) HashMap，u32 hash 快，节点数千量级可接受。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimTable(pub std::collections::HashMap<NodeId, NodeAnim>);

impl AnimTable {
    pub fn get(&self, node: NodeId) -> Option<&NodeAnim> {
        self.0.get(&node).filter(|a| !a.is_empty())
    }

    /// 确保该节点有 anim 槽并返回可变引用（update 调）。
    pub fn ensure(&mut self, node: NodeId) -> &mut NodeAnim {
        self.0.entry(node).or_insert_with(NodeAnim::default)
    }

    /// 清该节点所有通道（回 CSS）= remove。
    pub fn clear_node(&mut self, node: NodeId) {
        self.0.remove(&node);
    }

    /// 清该节点某 prop 对应通道（Translate/Scale/Rotation 都映射到 transform 通道）。
    pub fn clear_prop(&mut self, node: NodeId, prop: crate::tween::TweenProp) {
        let a = match self.0.get_mut(&node) {
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
    /// 节点存储。Vec<Node> → SlotMap<DefaultKey, Node>（v1.3+ 动态树 spec §4.1）。
    /// 应用层用 NodeId(u32) 句柄（FFI/C# 透明），经 `Scene::key_for`/`NodeId::to_key` 桥接到 DefaultKey。
    pub nodes: SlotMap<DefaultKey, Node>,
    /// 运行时伪类重匹配规则表。默认空；包加载填，inline 路径空。
    pub dynamic_rules: crate::style::dynamic::DynamicRuleTable,
    /// 当前焦点节点（单一全局，照 fgui Stage.focus）。None=无焦点。
    pub focused_node: Option<NodeId>,
    /// 每节点累计世界矩阵（compute_world_transforms 填）。index = NodeId.index()。运行时态，不进 pkg。
    pub world_transforms: Vec<crate::transform::Affine2>,
    /// 每节点动画 override（TweenManager.update 填）。index = NodeId.index()。运行时态，不进 pkg。
    pub anim: AnimTable,
    /// 每节点滚动状态（refresh_content_sizes / scroll 物理填）。index = NodeId.index()。运行时态，不进 pkg。
    pub scroll: crate::scroll::ScrollTable,
    /// 每节点 text 测量结果（layout solve 填，render 复用——消除双测量不一致）。
    /// index = NodeId.index()，仅 Text 节点 Some。运行时态，不进 pkg。
    ///
    /// 根因：layout 闭包用 taffy 选定 max_width 测（短文本 intrinsic≈available → taffy 传 None
    /// 不换行；长文本 → Some(available) 换行），render 若用 rect.w（stretch 后的 available 整数宽）
    /// 重测，短文本因 intrinsic 亚像素超 available 误判换行。故 render 复用 layout 结果，不重测。
    pub text_layouts: Vec<Option<crate::text::layout::TextLayout>>,
}

impl Scene {
    /// 从扁平 entries（DFS 先序）建 Node 树。`parent_idx` 指向 entries 下标，`None` = 根。
    /// clip_rect slot / dirty 标志按 style.overflow_x/y（非 Visible 即 clip）/ kind 派生。
    /// parse 路径（build_scene）与包加载路径（read_package）共用。
    ///
    /// **NodeId 由 slotmap 分配**：entries 第 i 个 → slotmap insert → NodeId（idx=i+1，version=1，
    /// 无删除时）。parent/children 用 entries 下标 → 经临时 ids 表映射到 NodeId。
    pub fn build(entries: &[(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)]) -> Scene {
        let mut scene = Scene {
            roots: Vec::new(),
            nodes: SlotMap::with_key(),
            dynamic_rules: crate::style::dynamic::DynamicRuleTable::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        // 先 insert 所有节点，收集 slotmap 分配的 NodeId
        let mut ids: Vec<NodeId> = Vec::with_capacity(entries.len());
        for (_, kind, style, classes, id_attr, draggable, tabindex) in entries.iter() {
            let node = Node {
                id: NodeId::INVALID, // 临时，insert 后回填
                parent: None, // 下一轮填
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
            };
            let key = scene.nodes.insert(node);
            let id = NodeId::from_key(key);
            scene.nodes.get_mut(key).unwrap().id = id; // 回填
            ids.push(id);
        }
        // 接 parent/children/roots（用 ids 映射 entries 下标 → NodeId）
        for (i, (parent_idx, _, _, _, _, _, _)) in entries.iter().enumerate() {
            match parent_idx {
                Some(p) => {
                    let child_id = ids[i];
                    let parent_id = ids[*p];
                    let ck = child_id.to_key();
                    let pk = parent_id.to_key();
                    scene.nodes.get_mut(ck).unwrap().parent = Some(parent_id);
                    scene.nodes.get_mut(pk).unwrap().children.push(child_id);
                }
                None => scene.roots.push(ids[i]),
            }
        }
        // text_layouts 随槽位容量对齐（None 占位，layout::solve 填实际 TextLayout）。
        // **容量而非存活数**（T5）：按 id.index() 索引，remove_node 后 idx 不变但存活数减，
        // 按 len 分配会越界。capacity+1（1 基索引，idx 0 占位）。
        scene.text_layouts = vec![None; scene.nodes.capacity() + 1];
        scene
    }

    /// test helper：从节点列表 + (parent_idx, child_idx) 边建 Scene。替代 70+ 字面量。
    /// roots = 无 parent 的节点（按插入序）。
    pub fn from_nodes(nodes: Vec<Node>, edges: Vec<(usize, usize)>) -> Scene {
        let mut scene = Scene {
            roots: Vec::new(),
            nodes: SlotMap::with_key(),
            dynamic_rules: crate::style::dynamic::DynamicRuleTable::default(),
            focused_node: None,
            world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new(),
        };
        let mut ids: Vec<NodeId> = Vec::with_capacity(nodes.len());
        for n in nodes {
            let key = scene.nodes.insert(n);
            let id = NodeId::from_key(key);
            scene.nodes.get_mut(key).unwrap().id = id;
            ids.push(id);
        }
        for (p, c) in edges {
            let pid = ids[p];
            let cid = ids[c];
            let pk = pid.to_key();
            let ck = cid.to_key();
            scene.nodes.get_mut(ck).unwrap().parent = Some(pid);
            scene.nodes.get_mut(pk).unwrap().children.push(cid);
        }
        // roots = 无 parent 的（按 ids 插入序）
        for &id in &ids {
            if scene.nodes.get(id.to_key()).unwrap().parent.is_none() {
                scene.roots.push(id);
            }
        }
        scene.text_layouts = vec![None; scene.nodes.capacity() + 1];
        scene
    }

    /// NodeId → slotmap DefaultKey 桥接（内部用）。
    pub fn key_for(&self, id: NodeId) -> DefaultKey {
        id.to_key()
    }

    /// 按 NodeId 取节点（slotmap get，自带 gen 校验，悬空返 None）。
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.to_key())
    }
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.to_key())
    }

    /// 按 CSS id 属性查节点（首个匹配）。无匹配 / 空 id → None。
    /// 供 FFI find_node_by_id：业务用 id 注册 listener / 设 disabled，替代硬编码 build 序 id
    /// （auto Text 子会偏移 build 序，硬编码不可靠）。
    pub fn find_by_id_attr(&self, id: &str) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|(_, n)| n.id_attr.as_deref() == Some(id))
            .map(|(_, n)| n.id)
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
    let el = &tree.nodes[el_id.0 as usize];
    let style = &styles[el_id.0 as usize];
    // tag→NodeKind 复用 runtime 的 `kind_from_tag`（dynamic.rs，不依赖 parse feature），
    // 消除两处 tag 白名单重复。parse 层已保证 tag 在围栏白名单内（div/span/img/button/l-container），
    // 故 kind_from_tag 在此必 Ok——Err 走 unreachable（parse/白名单契约破坏）。
    // kind_from_tag 对 img/span 返空 src/content（动态建树语义）；parse 路径需从元素属性/文本回填。
    // img 的 src 从属性取（`<img src="...">`），不是元素文本；
    // span 的文本是其自身 content（Text 叶子，无子节点）。
    let mut kind = crate::scene::dynamic::kind_from_tag(&el.tag)
        .unwrap_or_else(|_| unreachable!(
            "parse 层白名单已挡围栏外 tag，scene 不应见到 <{}>；这是 parse/scene 契约破坏",
            el.tag
        ));
    match &mut kind {
        NodeKind::Image { src } => {
            *src = el.attrs.get("src").cloned().unwrap_or_default();
        }
        NodeKind::Text { content } => {
            *content = el.text.clone().unwrap_or_default();
        }
        _ => {}
    }
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
    fn node_id_index_and_gen_decode() {
        // 高 20 bit index + 低 12 bit gen
        let id = NodeId((5 << 12) | 7);
        assert_eq!(id.index(), 5, "index = 高 20 bit");
        assert_eq!(id.gen(), 7, "gen = 低 12 bit");
    }

    #[test]
    fn node_id_invalid_sentinel() {
        assert!(!NodeId::INVALID.is_valid(), "0xFFFF_FFFF = INVALID");
        assert!(NodeId(0).is_valid(), "0 有效");
    }

    #[test]
    fn scene_nodes_is_slotmap_and_get_by_id() {
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), Vec::new(), None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        // slotmap get by NodeId
        let root_id = scene.roots[0];
        assert!(scene.nodes.get(root_id.to_key()).is_some(), "live NodeId 可 get（经 to_key）");
        assert!(scene.get(root_id).is_some(), "Scene::get 桥接可用");
        // get_mut
        if let Some(n) = scene.get_mut(root_id) {
            n.disabled = true;
        }
        assert!(scene.get(root_id).unwrap().disabled);
    }

    #[test]
    fn scene_from_nodes_helper_builds_tree() {
        // test helper：从 Vec<Node> 建 Scene（替代字面量）
        let root = Node::default();
        let child = Node::default();
        let scene = Scene::from_nodes(vec![root, child], vec![(0, 1)]); // (parent_idx, child_idx)
        assert_eq!(scene.nodes.len(), 2);
        assert_eq!(scene.roots.len(), 1, "root 无 parent → roots 1 个");
        let root_id = scene.roots[0];
        let root_node = scene.get(root_id).unwrap();
        assert_eq!(root_node.children.len(), 1, "root 有 1 child");
        let child_id = root_node.children[0];
        assert_eq!(scene.get(child_id).unwrap().parent, Some(root_id));
    }

    #[test]
    fn node_id_from_key_to_key_roundtrip() {
        // 验证 NodeId ↔ DefaultKey 桥接 roundtrip（version=1，T2 无删除）
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
        ];
        let scene = Scene::build(&entries);
        let id = scene.roots[0];
        // to_key 后 slotmap 能查到
        assert!(scene.nodes.get(id.to_key()).is_some(), "to_key 重构的 key 能查到节点");
        // index = slotmap idx = 1（slotmap free_head 从 1 起，idx 0 是 sentinel）
        assert_eq!(id.index(), 1, "首节点 slotmap idx=1");
        assert_eq!(id.gen(), 1, "version=1（无删除）");
    }

    #[test]
    fn node_id_index_capacity_20bit() {
        // 20 bit index 上限 = (1<<20)-1 = 1048575
        let max_idx = (1u32 << 20) - 1;
        let id = NodeId(max_idx << 12);
        assert_eq!(id.index(), max_idx as usize);
    }

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
        let root_id = scene.roots[0];
        let btn_id = scene.get(root_id).unwrap().children[0];
        assert!(!scene.get(root_id).unwrap().draggable, "root draggable=false");
        assert!(scene.get(btn_id).unwrap().draggable, "btn draggable=true");
    }

    #[test]
    fn scene_default_has_empty_dynamic_rules() {
        let s = Scene {
            roots: vec![],
            nodes: SlotMap::with_key(),
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
            nodes: SlotMap::with_key(),
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
        let root_id = scene.roots[0];
        let kids = &scene.get(root_id).unwrap().children;
        let btn1 = kids[0];
        let btn2 = kids[1];
        assert_eq!(scene.get(root_id).unwrap().tabindex, None, "root tabindex=None");
        assert_eq!(scene.get(btn1).unwrap().tabindex, Some(0), "btn1 tabindex=Some(0)");
        assert_eq!(scene.get(btn2).unwrap().tabindex, Some(3), "btn2 tabindex=Some(3)");
        assert!(!scene.get(root_id).unwrap().focused, "focused 默认 false");
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
        assert_eq!(scene.roots.len(), 1, "根 = parent=None 的节点");
        let root_id = scene.roots[0];
        let root = scene.get(root_id).unwrap();
        assert!(matches!(root.kind, NodeKind::Container));
        assert_eq!(root.children.len(), 1, "Text 子挂 root");
        let text_id = root.children[0];
        assert!(root.clip_rect.is_none(), "overflow Visible → 无 clip slot");
        assert!(!root.dirty_text, "Container dirty_text=false");
        let text = scene.get(text_id).unwrap();
        assert!(matches!(&text.kind, NodeKind::Text { content } if content == "hi"));
        assert_eq!(text.parent, Some(root_id));
        assert!(text.dirty_text, "Text 节点 dirty_text=true");

        // overflow Hidden → clip slot 派生
        let mut of = ResolvedStyle::default();
        of.overflow_x = OverflowMode::Hidden;
        of.overflow_y = OverflowMode::Hidden;
        let scene2 = Scene::build(&[(None, NodeKind::Container, of, Vec::new(), None, false, None)]);
        assert!(scene2.get(scene2.roots[0]).unwrap().clip_rect.is_some(), "overflow Hidden → clip slot");
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
            assert!(sc.get(sc.roots[0]).unwrap().clip_rect.is_some(), "{} → clip slot", desc);
        }
        // 双轴 Visible → 无 clip slot（对照）
        let mut vis = ResolvedStyle::default();
        vis.overflow_x = OverflowMode::Visible;
        vis.overflow_y = OverflowMode::Visible;
        let sc = Scene::build(&[(None, NodeKind::Container, vis, Vec::new(), None, false, None)]);
        assert!(sc.get(sc.roots[0]).unwrap().clip_rect.is_none(), "双轴 Visible → 无 clip slot");
    }

    /// AnimTable 用 HashMap<NodeId, NodeAnim>（T3）。测试一律用 slotmap 分配的真实 NodeId
    /// + 生产路径写法（ensure(id)），不用字面量 NodeId(N) 撑表（reviewer Minor-3）。
    fn anim_scene_one_node() -> (Scene, NodeId) {
        let sc = Scene::build(&[(None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None)]);
        let id = sc.roots[0];
        (sc, id)
    }

    #[test]
    fn animtable_hashmap_get_ensure_clear() {
        let (_sc, id) = anim_scene_one_node();
        let mut t = AnimTable::default();
        // 未 ensure 的 id → get None
        assert!(t.get(id).is_none(), "未 ensure → get None");
        // ensure + 写
        t.ensure(id).opacity = Some(0.5);
        assert_eq!(t.get(id).unwrap().opacity, Some(0.5));
        // 全默认的 NodeAnim（ensure 后未写任何通道）→ get 返 None（is_empty 过滤）
        let other = {
            let sc = Scene::build(&[
                (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
                (Some(0), NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            ]);
            sc.roots[0]
        };
        // 注：other 是另一 scene 的 NodeId，此处仅验证 ensure 后未写 → get None
        let mut t2 = AnimTable::default();
        t2.ensure(other);
        assert!(t2.get(other).is_none(), "ensure 后全 None → is_empty 过滤 → get None");
        // clear_node
        t.clear_node(id);
        assert!(t.get(id).is_none(), "clear_node 后 get 返 None");
    }

    #[test]
    fn animtable_clear_prop_keeps_other_channels() {
        let (_sc, id) = anim_scene_one_node();
        let mut t = AnimTable::default();
        let a = t.ensure(id);
        a.opacity = Some(0.5);
        a.transform = Some(crate::transform::from_scale(2.0, 2.0));
        t.clear_prop(id, crate::tween::TweenProp::Scale);
        assert!(t.get(id).unwrap().transform.is_none(), "清 transform 通道");
        assert_eq!(t.get(id).unwrap().opacity, Some(0.5), "opacity 通道保留");
    }

    #[test]
    fn animtable_clear_prop_all_variants() {
        let (_sc, id) = anim_scene_one_node();
        let mut t = AnimTable::default();
        let a = t.ensure(id);
        a.opacity = Some(0.5);
        a.transform = Some(crate::transform::from_scale(2.0, 2.0));
        a.bg_color = Some([1.0; 4]);
        a.text_color = Some([2.0; 4]);
        // 注：clear_prop 后断言用 t.0.get(&id)（绕过 get 的 is_empty 过滤），
        // 因逐通道清到全 None 时 get 会返 None，但条目本身仍在（clear_node 才 remove）。
        // macro：每次展开独立借用，避免闭包持借冲突 clear_prop 的 &mut。
        macro_rules! raw { () => { t.0.get(&id).expect("条目存在（clear_prop 不 remove）") }; }
        t.clear_prop(id, crate::tween::TweenProp::Opacity);
        assert!(raw!().opacity.is_none(), "清 opacity");
        assert!(raw!().transform.is_some(), "opacity 清了，transform 保留");
        t.clear_prop(id, crate::tween::TweenProp::Translate);
        assert!(raw!().transform.is_none(), "Translate 清 transform");
        // 重新写 transform 再清 Scale/Rotation
        t.ensure(id).transform = Some(crate::transform::from_scale(2.0, 2.0));
        t.clear_prop(id, crate::tween::TweenProp::Scale);
        assert!(raw!().transform.is_none(), "Scale 清 transform");
        t.ensure(id).transform = Some(crate::transform::from_rotate(0.5));
        t.clear_prop(id, crate::tween::TweenProp::Rotation);
        assert!(raw!().transform.is_none(), "Rotation 清 transform");
        t.clear_prop(id, crate::tween::TweenProp::BgColor);
        assert!(raw!().bg_color.is_none(), "清 bg_color");
        t.clear_prop(id, crate::tween::TweenProp::TextColor);
        assert!(raw!().text_color.is_none(), "清 text_color");
        // 全清后 → is_empty → get None（条目仍在，但 get 过滤掉）
        assert!(t.get(id).is_none(), "全通道清后 get 返 None（is_empty 过滤）");
        // clear_node 才真正 remove
        t.clear_node(id);
        assert!(t.0.get(&id).is_none(), "clear_node 后 HashMap 无条目");
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
        let root = scene.get(scene.roots[0]).unwrap();
        assert_eq!(root.classes, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(root.id_attr.as_deref(), Some("x"));
        let span = scene.get(root.children[0]).unwrap();
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
        let root_id = scene.roots[0];
        let btn_id = scene.get(root_id).unwrap().children[0];
        assert_eq!(scene.find_by_id_attr("btn"), Some(btn_id), "find btn → btn node");
        assert_eq!(scene.find_by_id_attr("root"), Some(root_id), "find root → root node");
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
        let root = scene.get(scene.roots[0]).unwrap();
        assert!(matches!(root.kind, NodeKind::Container));
        assert_eq!(root.children.len(), 3);
        let c0 = root.children[0];
        let c1 = root.children[1];
        let c2 = root.children[2];
        assert!(matches!(scene.get(c0).unwrap().kind, NodeKind::Button));
        let text = scene.get(c1).unwrap();
        match &text.kind {
            NodeKind::Text { content } => assert_eq!(content, "hi"),
            _ => panic!("expected Text"),
        }
        match &scene.get(c2).unwrap().kind {
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
        assert!(scene.get(scene.roots[0]).unwrap().clip_rect.is_some());
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
        let root = scene.get(scene.roots[0]).unwrap();
        match &scene.get(root.children[0]).unwrap().kind {
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
        let root = scene.get(scene.roots[0]).unwrap();
        assert!(root.dirty_mesh);
        assert!(!root.dirty_text); // Container 不脏文本
        let text_id = root.children[0];
        let text = scene.get(text_id).unwrap();
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
        let root = scene.get(scene.roots[0]).unwrap();
        assert!(matches!(root.kind, NodeKind::Container));
        assert_eq!(root.children.len(), 1, "裸文本应产 1 个 Text 子节点");
        let child_id = root.children[0];
        let child = scene.get(child_id).unwrap();
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
        let btn = scene.get(scene.roots[0]).unwrap();
        assert!(matches!(btn.kind, NodeKind::Button));
        assert_eq!(btn.children.len(), 1);
        match &scene.get(btn.children[0]).unwrap().kind {
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
        let root = scene.get(scene.roots[0]).unwrap();
        let child = scene.get(root.children[0]).unwrap();
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
        let root = scene.get(scene.roots[0]).unwrap();
        // btn 是 root 的子（root 的 Text 子"OK"另算——button 裸文本→Text 子）。
        // 找 button kind 的子：
        let btn = scene.nodes.values().find(|n| matches!(n.kind, NodeKind::Button)).expect("btn");
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
        let root = scene.get(scene.roots[0]).unwrap();
        assert!(!root.draggable, "draggable=\"false\" → false");
        let btn = scene.nodes.values().find(|n| matches!(n.kind, NodeKind::Button)).expect("btn");
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
        let btns: Vec<_> = scene.nodes.values().filter(|n| matches!(n.kind, NodeKind::Button)).collect();
        assert_eq!(btns.len(), 5);
        assert_eq!(btns[0].tabindex, Some(0), "tabindex=\"0\" → Some(0)");
        assert_eq!(btns[1].tabindex, Some(3), "tabindex=\"3\" → Some(3)");
        assert_eq!(btns[2].tabindex, Some(-1), "tabindex=\"-1\" → Some(-1)");
        assert_eq!(btns[3].tabindex, None, "tabindex=\"abc\"（非数字）→ None");
        assert_eq!(btns[4].tabindex, None, "无 tabindex 属性 → None");
    }
}
