//! 运行时伪类匹配的动态规则层。
//!
//! 本模块实现 \match_element_with_state\：选择器匹配 + 伪类状态标 +
//! ematch_pseudo_classes\，对所有节点做重算（写 Node.style + 标 layout dirty）。
//!
//! 类型模型（\ParsedSelector\/\Compound\/\Combinator\/\Specificity\）+ \Declaration\（CSS 声明）+
//! \compound_matches_node\（运行时 compound 匹配）+ 动态规则匹配全部无条件编译——
//! bincode 反序列化的 \.pkg.bin\ 就是这些结构，runtime 不再 parse 选择器，直接用反序列化结构。
//! 字符串 → 这些结构的解析器在 fence crate（\yio_fence\）。

use serde::{Deserialize, Serialize};

/// CSS 声明（prop + value），序列化进 .pkg.bin DynamicRuleSection。
/// \PartialEq\ 用于 instantiate 伪类去重（同选择器 + 同声明视为重复，跳过）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Declaration {
    pub prop: String,
    pub value: String,
}

/// 选择器组合子：标签/类/id/后代/子代 + 伪类状态门（hover/active/disabled）。
/// `PartialEq` 供 instantiate 规则去重（结构相等 = 同选择器，含 raw/compound/specificity）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedSelector {
    pub raw: String,
    pub compound: Vec<Compound>, // 复合选择器链（后代/子代分隔）
    pub specificity: Specificity,
}

/// `:nth-child(An+B)` 表达式参数：`odd`=`(2,1)`、`even`=`(2,0)`、纯整数 N=`(0,N)`、
/// `An+B`=`(A,B)`。`a == 0` 时仅匹配第 `b` 个子节点（纯整数形态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NthChildExpr {
    pub a: i32,
    pub b: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compound {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub id: Option<String>,
    pub combinator: Combinator, // 本 compound 与前一个的关系
    pub pseudo_hover: bool,
    pub pseudo_active: bool,
    pub pseudo_disabled: bool,
    pub pseudo_focus: bool,
    /// `:nth-child(...)` 参数（None = 未声明）。结构伪类：匹配依赖节点在父
    /// `children` 的位置（1-based index），见 `nth_child_matches`。
    pub pseudo_nth_child: Option<NthChildExpr>,
    /// 属性选择器（`[attr]` / `[attr="val"]`）。出现属性选择器即把规则划入动态规则表
    /// （运行时按节点 attrs 匹配，静态 cascade 无法预判），由 compound_matches_node 匹配。
    pub attrs: Vec<AttrSelector>,
    /// `::part(name)` 载体（#57）：本 compound 的其余字段（tag/classes/id/attrs/伪类）
    /// 匹配**组件 host**，part 匹配 host 展开子树内带 `part="name"` 属性的**目标节点**
    /// （一层——嵌套组件内部不递归，其作用域根是嵌套 host）。必须是最后一个 compound
    /// 且位于其结尾（fence 解析期保证）。None = 普通选择器。runtime 注入（#11 全局规则
    /// 字面穿墙）与本臂正交：::part 是打包期页面规则穿组件内容墙的唯一许可通道。
    pub part: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Combinator {
    Descendant,
    Child,
}

/// 属性选择器运算符（围栏子集：仅存在性 + 相等；不做 ~=, ^=, $=, *=）。
/// Copy：无字段枚举，运行时 attr 匹配按值分派（Eq/Exists），Copy 使其在元组 match 等
/// 场景零成本取用，无需克隆。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttrOp {
    /// `[attr]` — 属性存在即匹配。
    Exists,
    /// `[attr="val"]` — 属性值字面相等。
    Eq,
}

/// 单条属性选择器（如 `[data-page="1"]`）。name 用小写归一（HTML 属性名大小写不敏感）。
/// value 仅 Eq 有意义；Exists 时为 None。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttrSelector {
    pub name: String,
    pub op: AttrOp,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Specificity(pub u32, pub u32, pub u32); // (id 数, class 数, tag 数)

use crate::scene::control::ROLE_TAB;
use crate::scene::node::{NodeFlags, NodeId, Scene};
use crate::style::mapping::apply_decl;
use crate::style::resolved::{AnimationSpec, DisplayMode, ResolvedStyle, TransitionSpec};
use std::collections::HashMap;

use crate::style::resolved::InheritedSet;

const INH_FONT_SIZE: u64 = 1 << 0;
const INH_COLOR: u64 = 1 << 1;
const INH_FONT_FAMILY: u64 = 1 << 2;
const INH_FONT_WEIGHT: u64 = 1 << 3;
const INH_TEXT_ALIGN: u64 = 1 << 4;
const INH_LINE_HEIGHT: u64 = 1 << 5;
const INH_LETTER_SPACING: u64 = 1 << 6;
const INH_WHITE_SPACE: u64 = 1 << 7;
// #73 起的新继承属性：bits 8-32 已被 INLINE_* 非继承属性占用，INH_* 扩到 bits 33+
// （InheritedSet/InlineSet 均为 u64，同一位空间；serial 侧 InheritedSet 随 v45 拓宽）。
const INH_OVERFLOW_WRAP: u64 = 1 << 33;
const INH_WORD_BREAK: u64 = 1 << 34;
const INH_TEXT_WRAP: u64 = 1 << 35;

/// prop 名 → 可继承属性 bit（非可继承返 None）。单一真相源：bit 的定义（本表 INH_*）与
/// 消费（rematch set bit + propagate copy_if_unset）都在 core。fence css_resolve 调本函数
/// 把 inline 可继承声明 bake 进 ResolvedStyle.inherited_set，避免运行时被父值覆盖。
pub fn inherited_bit(prop: &str) -> Option<u64> {
    match prop.trim() {
        "font-size" => Some(INH_FONT_SIZE),
        "color" => Some(INH_COLOR),
        "font-family" => Some(INH_FONT_FAMILY),
        "font-weight" => Some(INH_FONT_WEIGHT),
        "text-align" => Some(INH_TEXT_ALIGN),
        "line-height" => Some(INH_LINE_HEIGHT),
        "letter-spacing" => Some(INH_LETTER_SPACING),
        "white-space" => Some(INH_WHITE_SPACE),
        "overflow-wrap" => Some(INH_OVERFLOW_WRAP),
        "word-break" => Some(INH_WORD_BREAK),
        "text-wrap" => Some(INH_TEXT_WRAP),
        _ => None,
    }
}

// InlineSet 与 InheritedSet 同构（newtype 包位图），但语义相反：
//   - InheritedSet: 打包期 bake 进 base_style.inherited_set，序列化进 pkg.bin
//   - InlineSet:    运行时 transient，C# Style.X=v 写入，不进 pkg.bin
// 继承属性 bit 复用 INH_*（同一位空间，不重新编号）；非继承属性用 INLINE_*。
//
// **位编号说明：** INH_* 占用 bits 0-7（8 个早期继承属性）+ bits 33-35（#73 的
// overflow-wrap/word-break/text-wrap——8-32 被 INLINE_* 占用故越过）。bits 8-31 共
// 24 位容纳了 apply_decl 处理的 24 个非继承属性（width/height/min-*/max-*/padding/
// margin/border-width/gap/flex-*/display/overflow-x/y/position/left/top/right/bottom/
// background-color/opacity）。u32 装满后位图升级为 u64：z-index 取 bit 32，
// text-decoration 取 bit 36（#74 非继承属性；33-35 归 INH_*），bits 37-63 仍空。

/// inline override 的 set-ness 位图。复用 INH_* 给继承属性（bits 0-7 + 33-35），
/// 其后是 INLINE_* 非继承属性 bit。rematch 用它应用便签层；继承子集 OR 进 set_map
/// 让 propagate 自动传播父的 inline 继承值给未自设的子。纯运行时 transient，不进 pkg.bin。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InlineSet(pub u64);

/// 所有继承属性 bit 的 OR——rematch 用它把 inline 的继承部分并进 set_map，
/// 使 propagate_inherited 把父的 inline 继承值（如 inline color）传给未自设的子。
pub const INH_ALL_MASK: u64 = INH_FONT_SIZE
    | INH_COLOR
    | INH_FONT_FAMILY
    | INH_FONT_WEIGHT
    | INH_TEXT_ALIGN
    | INH_LINE_HEIGHT
    | INH_LETTER_SPACING
    | INH_WHITE_SPACE
    | INH_OVERFLOW_WRAP
    | INH_WORD_BREAK
    | INH_TEXT_WRAP;

// 非继承属性 bit（编号接在 INH_* 之后，从 bit 8 起）。对照 apply_decl 能处理的属性清单，
// 逐个分配 1 bit。INH_* 位（继承属性）复用 inherited_bit，不重复定义。
pub const INLINE_WIDTH: u64 = 1 << 8;
pub const INLINE_HEIGHT: u64 = 1 << 9;
pub const INLINE_MIN_WIDTH: u64 = 1 << 10;
pub const INLINE_MIN_HEIGHT: u64 = 1 << 11;
pub const INLINE_MAX_WIDTH: u64 = 1 << 12;
pub const INLINE_MAX_HEIGHT: u64 = 1 << 13;
pub const INLINE_PADDING: u64 = 1 << 14;
pub const INLINE_MARGIN: u64 = 1 << 15;
pub const INLINE_BORDER_WIDTH: u64 = 1 << 16;
pub const INLINE_GAP: u64 = 1 << 17;
pub const INLINE_FLEX_DIRECTION: u64 = 1 << 18;
pub const INLINE_FLEX_WRAP: u64 = 1 << 19;
pub const INLINE_JUSTIFY_CONTENT: u64 = 1 << 20;
pub const INLINE_ALIGN_ITEMS: u64 = 1 << 21;
pub const INLINE_DISPLAY: u64 = 1 << 22;
pub const INLINE_OVERFLOW_X: u64 = 1 << 23;
pub const INLINE_OVERFLOW_Y: u64 = 1 << 24;
pub const INLINE_POSITION: u64 = 1 << 25;
pub const INLINE_LEFT: u64 = 1 << 26;
pub const INLINE_TOP: u64 = 1 << 27;
pub const INLINE_RIGHT: u64 = 1 << 28;
pub const INLINE_BOTTOM: u64 = 1 << 29;
pub const INLINE_BACKGROUND_COLOR: u64 = 1 << 30;
pub const INLINE_OPACITY: u64 = 1 << 31;
/// z-index（层叠序）。u32 位图装满后升级 u64 的首个扩展位。
pub const INLINE_Z_INDEX: u64 = 1 << 32;
/// text-decoration（#74 `<a>` UA underline 的作者 inline 覆盖保护）。bits 33-35 归
/// INH_*（#73 继承属性），故越过到 bit 36。
pub const INLINE_TEXT_DECORATION: u64 = 1 << 36;
/// cursor（#93 桌面指针 affordance 的作者 inline 覆盖）。
pub const INLINE_CURSOR: u64 = 1 << 37;

/// prop 名 → InlineSet bit。继承属性复用 `inherited_bit`（bits 0-7），非继承属性走
/// INLINE_*（bits 8-31，z-index 在 bit 32、text-decoration 在 bit 36、cursor 在 bit 37）。
/// 返回 None = 该属性不可 inline（apply_decl 也不处理）。
///
/// **覆盖范围：** apply_decl 处理的所有非继承属性都有 bit（对照
/// `crates/core/src/style/mapping.rs::apply_decl`）。inset 四边（top/right/bottom/left）
/// 各占独立 bit（虽由 position 派生，但 C# Style API 暴露为 4 个独立 Length setter）。
/// 少数装饰性/列表型属性（transition / text-shadow / -webkit-text-stroke / font-effect /
/// box-shadow / background-image / background-size / border-color / border-radius /
/// transform / order / pointer-events / background-clip）不在 inline 范围：
/// 它们要么是列表（Vec）不便简单 set/unset，要么已有独立路径（transform 走 NodeAnim），
/// 要么设计期声明为主（bg-image 等）。这些若后续需要 inline，再扩位图。
pub fn inline_bit(prop: &str) -> Option<u64> {
    if let Some(b) = inherited_bit(prop) {
        return Some(b);
    }
    match prop.trim() {
        "width" => Some(INLINE_WIDTH),
        "height" => Some(INLINE_HEIGHT),
        "min-width" => Some(INLINE_MIN_WIDTH),
        "min-height" => Some(INLINE_MIN_HEIGHT),
        "max-width" => Some(INLINE_MAX_WIDTH),
        "max-height" => Some(INLINE_MAX_HEIGHT),
        "padding" => Some(INLINE_PADDING),
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => Some(INLINE_PADDING),
        "margin" => Some(INLINE_MARGIN),
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => Some(INLINE_MARGIN),
        "border-width" => Some(INLINE_BORDER_WIDTH),
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            Some(INLINE_BORDER_WIDTH)
        }
        "gap" => Some(INLINE_GAP),
        "flex-direction" => Some(INLINE_FLEX_DIRECTION),
        "flex-wrap" => Some(INLINE_FLEX_WRAP),
        "justify-content" => Some(INLINE_JUSTIFY_CONTENT),
        "align-items" => Some(INLINE_ALIGN_ITEMS),
        "display" => Some(INLINE_DISPLAY),
        "overflow-x" => Some(INLINE_OVERFLOW_X),
        "overflow-y" => Some(INLINE_OVERFLOW_Y),
        "position" => Some(INLINE_POSITION),
        "left" => Some(INLINE_LEFT),
        "top" => Some(INLINE_TOP),
        "right" => Some(INLINE_RIGHT),
        "bottom" => Some(INLINE_BOTTOM),
        "background-color" => Some(INLINE_BACKGROUND_COLOR),
        "opacity" => Some(INLINE_OPACITY),
        "z-index" => Some(INLINE_Z_INDEX),
        "text-decoration" => Some(INLINE_TEXT_DECORATION),
        "cursor" => Some(INLINE_CURSOR),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DynamicRuleTable {
    pub rules: Vec<DynamicRule>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DynamicRule {
    pub selector: ParsedSelector,
    pub declarations: Vec<Declaration>,
}

/// 带作用域的动态规则（scene 运行时态，不进 pkg）。Shadow DOM 风格：
/// 模板实例化时，规则绑定到实例根 NodeId；rematch 只在 scope 内匹配 + 后代选择器不穿透边界。
/// `scope_root == NodeId::INVALID` = 全局规则（UIContext.StyleSheet 逃生舱），跨作用域命中。
/// pkg 里的 DynamicRuleTable（无 scope）在 instantiate 时包装成 ScopedRule。
#[derive(Debug, Clone)]
pub struct ScopedRule {
    pub rule: DynamicRule,
    pub scope_root: crate::scene::node::NodeId,
}

/// scene 级规则容器（运行时）。pkg 用 DynamicRuleTable（无 scope，可序列化）。
#[derive(Debug, Clone, Default)]
pub struct ScopedRuleTable {
    pub entries: Vec<ScopedRule>,
}

/// 一批运行时注入的规则（`UIContext.StyleSheet.Add` 通道，#11）。id = 撤销句柄
/// （C# Dispose 按它 remove）。规则按全局语义参与 rematch（scope_root=INVALID，
/// 跨作用域命中——见 `Scene::runtime_rule_sets` 字段注释的墙语义说明）。
#[derive(Debug, Clone)]
pub struct RuntimeRuleSet {
    pub id: u64,
    pub rules: Vec<DynamicRule>,
}

use crate::scene::node::{ControlState, NodeKind};

/// 运行时版 compound 匹配（消费 Node 而非 ElementData，运行时无 ElementTree）。
/// 匹配 tag/classes/id（不含伪类状态——状态由 match_element_with_state 门控）。
/// id 属性存 Node.id_attr（`id="..."`）；Node.id 是 NodeId 索引身份，二者不同。
///
/// **常驻：**runtime rematch 用，不依赖 parse feature。
pub fn compound_matches_node(c: &Compound, node_id: NodeId, scene: &Scene) -> bool {
    let node = scene.get_live(node_id, "dynamic/compound_matches_node");
    if let Some(t) = &c.tag {
        // NodeKind → HTML 标签名：标准元素用其 tag，控件 kind 回溯到作者写的 tag
        // （input/progress），使 `input[type="range"]`、`progress` 等选择器在运行时 rematch
        // 仍能命中。与 fence schema/tag.rs resolve_semantic（tag→SemanticKind→NodeKind）互逆：
        // fence 列出的、且在运行时成为 Node 的每个 tag 都有对应 arm，使任何通过围栏的 tag
        // 选择器在 rematch 仍命中。
        //
        // tag 匹配值：CustomElement 有 custom_tag 时用原始 hyphen 字面值（pkg v35 组件展开
        // 保留；`game-item-card { ... }` 选择器命中 host），否则按 kind 逆映射。
        let kind_tag = match node.custom_tag.as_deref() {
            Some(tag) => tag,
            None => match node.kind {
                NodeKind::Container => "div",
                NodeKind::Button => "button",
                NodeKind::Image => "img",
                NodeKind::TextNode | NodeKind::TextElement => "span",
                NodeKind::TextArea => "textarea",
                NodeKind::Dropdown => "select",
                NodeKind::OptionItem => "option",
                NodeKind::ListItem => "li",
                NodeKind::ListView => "ul",
                NodeKind::Slot => "slot",
                // <template> 已进场景树（强制 display:none），保留逆映射使 `template` tag 选择器可命中。
                NodeKind::Template => "template",
                // role 驱动控件（TabList/Tab）无专属 tag，逆映射取最接近的宿主 tag：
                //  - TabList → div（容器宿主，与 ListView 同）；
                //  - Tab → button（settings.html 用 <button role=tab>，点击语义同 Button）。
                // 作者几乎不靠 tag 选择器选 tab，role/class 选择器不受影响。
                NodeKind::TabList => "div",
                NodeKind::Tab => "button",
                // role 驱动的树控件（#8）同 TabList 宿主 div（作者写 <div role=tree/treeitem>）。
                NodeKind::Tree | NodeKind::TreeItem => "div",
                // `<a>` 链接：tag 选择器 `a { ... }` / `a:hover { ... }` 命中 Link 节点。
                NodeKind::Link => "a",
                // input 变体：type 在 parse 期固化为独立 kind，tag 统一为 "input"
                NodeKind::TextField
                | NodeKind::NumberField
                | NodeKind::Slider
                | NodeKind::Toggle
                | NodeKind::RadioButton => "input",
                NodeKind::ProgressBar => "progress",
                // CustomElement 无 custom_tag（动态建树等未带字面值路径）退回 div 宿主。
                NodeKind::CustomElement => "div",
            },
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
    // 属性选择器：attr_matches_node 按 name 分派——[type=] 查 NodeKind、[aria-*] 合成
    // ControlState 实时值、[role=]/[data-slot=] 查 RoleTable。Node 不携带任意 HTML 属性
    // 字面值，故非这几类的 attr 不命中。
    for a in &c.attrs {
        if !attr_matches_node(scene, node_id, a) {
            return false;
        }
    }
    if let Some(expr) = &c.pseudo_nth_child {
        if !nth_child_matches(scene, node_id, expr) {
            return false;
        }
    }
    true
}

/// `:nth-child(An+B)` 匹配：节点在父 `children` 的 1-based index `i`，
/// `a == 0` 时 `i == b`；否则 `(i - b) % a == 0 && (i - b) / a >= 0`。
/// 根节点（无父）不匹配任何 :nth-child。
fn nth_child_matches(scene: &Scene, node_id: NodeId, expr: &NthChildExpr) -> bool {
    let parent = match scene.get(node_id).and_then(|n| n.parent) {
        Some(p) => p,
        None => return false,
    };
    // CSS :nth-child 只数元素子。匿名文本叶（TextNode，如元素间换行/空白）不是元素，
    // 计入会让后续元素整体偏位、:nth-child(N) 失配（home nav-grid 实例：按钮间空白把
    // 7 张 card 挤到 2/4/6/8/10/12/14，:nth-child(1..7) 只命中 3 张）。
    let i = match scene.get(parent).and_then(|p| {
        p.children
            .iter()
            .filter(|&&c| scene.get(c).is_some_and(|n| n.kind != NodeKind::TextNode))
            .position(|&c| c == node_id)
    }) {
        Some(pos) => pos as i32 + 1, // 0-based → 1-based
        None => return false,
    };
    let a = expr.a;
    let b = expr.b;
    if a == 0 {
        i == b
    } else {
        let d = i - b;
        d % a == 0 && d / a >= 0
    }
}

/// 运行时属性选择器匹配。Node 不存任意 HTML 属性字面值，按 name 分派到三类来源：
/// - `[type="x"]`：role 化重构后控件由 `role` 驱动，但 `[type=...]` 选择器作为便利层保留——selector 值 → NodeKind 精确对应（`type_matches_nodekind`），如 `[type="checkbox"]` 命中 Toggle。
/// - `[aria-*]`：aria 实时值从 ControlState 合成（`synth_aria_value`），随控件状态变。
/// - `[role="x"]` / `[data-slot="x"]`：从 RoleTable 查打包期提取的静态值。
///
/// 其他 attr name 不命中（Node 无对应字面值）。Exists 与 Eq 均支持：Exists 判该 attr 对本
/// 节点是否有语义（如 `[aria-checked]` 对 Toggle 有、对普通 div 无），Eq 判合成/查表值 == 字面值。
fn attr_matches_node(scene: &Scene, id: NodeId, a: &AttrSelector) -> bool {
    let name = a.name.as_str();
    // aria-* 实时合成（值随 ControlState 变）
    if let Some(rest) = name.strip_prefix("aria-") {
        return match a.op {
            AttrOp::Eq => synth_aria_value(scene, id, rest).as_deref() == a.value.as_deref(),
            AttrOp::Exists => synth_aria_value(scene, id, rest).is_some(),
        };
    }
    match (name, a.op) {
        ("role", AttrOp::Eq) => scene.roles.role_of(id) == a.value.as_deref(),
        ("role", AttrOp::Exists) => scene.roles.role_of(id).is_some(),
        ("data-slot", AttrOp::Eq) => a
            .value
            .as_deref()
            .is_some_and(|v| scene.roles.slot_of(id, v).is_some()),
        ("data-slot", AttrOp::Exists) => scene.roles.get(id).is_some_and(|i| !i.slots.is_empty()),
        ("type", AttrOp::Eq) => match &a.value {
            Some(val) => type_matches_nodekind(scene, id, val),
            None => false,
        },
        // [type] 存在形式不支持：type 不再是结构性属性（input 已下线，控件走 role），
        // 且 Node 不存 type 字面值，存在性无匹配意义。
        ("type", AttrOp::Exists) => false,
        _ => false,
    }
}

/// `[type="x"]` selector 值 → NodeKind 精确匹配（runtime 选择器便利层）。
///
/// role 化重构后控件由 `role` 驱动（`<div role="switch">` → Toggle），但作者仍可写
/// `[type="checkbox"]` 这类属性选择器——本函数把选择器值映射到对应 NodeKind（`checkbox` →
/// Toggle、`range` → Slider、`text` → TextField ...）与节点实际 kind 比对。映射表是 selector
/// 便利层，不再与 fence `resolve_semantic`（已无 input 分支）耦合。
fn type_matches_nodekind(scene: &Scene, id: NodeId, val: &str) -> bool {
    let Some(node) = scene.get(id) else {
        return false;
    };
    let expected = match val {
        "text" => NodeKind::TextField,
        "number" => NodeKind::NumberField,
        "range" => NodeKind::Slider,
        "checkbox" => NodeKind::Toggle,
        "radio" => NodeKind::RadioButton,
        _ => return false,
    };
    node.kind == expected
}

/// 从 ControlState 合成 aria 属性的实时值（运行时随控件状态变）。Node 不存 HTML 属性字面值，
/// aria-* 的当前值必须由控件状态派生。返回 None = 该 aria 对本节点无语义（选择器不命中）。
///
/// - `aria-checked`：Toggle / Radio 的 `checked`（"true"/"false"）。
/// - `aria-expanded`：Dropdown 的 `open`；Tree branch 条目的 `expanded`（#8）。
/// - `aria-valuenow`：Progress / Slider 的 `value`（f32）或 NumberField 的数值文本。
/// - `aria-valuemin`：Progress / Slider 的 `min`（f32；运行时改 min 后属性选择器同拍镜像）。
/// - `aria-indeterminate`：Progress 的 `indeterminate`（"true"/"false"；入口 = 打包期
///   `aria-valuenow` 缺席的 ARIA 语义，运行时 API 可翻转）。
/// - `aria-multiline`：**静态**，按 NodeKind（TextArea="true"），不查 ControlState——TextArea
///   与 TextField 共用 EditState，多行属性由标签（textarea vs input）决定而非运行时状态。
/// - `aria-selected`：**跨节点**——Tab 无自身 ControlState，选中态从父 TabList.selected_index
///   派生（Tab 在 TabList 的 role=tab 子里的 0 基序号 == selected_index → "true"，否则 "false"）。
///   TreeItem 同理从所属 Tree.selected 的 NodeId 恒等派生（#8）。分支须在下面
///   `let cs = scene.controls.get(id)?` 之前，否则 Tab / leaf TreeItem 因无 cs 直接返 None。
/// - `aria-level`：TreeItem 的层级（顶层=1，#8）——结构派生（嵌套深度即层级），不依赖
///   ControlState。作者用 [aria-level="2"] 一类选择器做缩进层级样式。
fn synth_aria_value(scene: &Scene, id: NodeId, aria: &str) -> Option<String> {
    // aria-multiline 静态：按 NodeKind 判（TextArea vs TextField），不依赖 ControlState。
    if aria == "multiline" {
        return match scene.get(id).map(|n| n.kind) {
            Some(NodeKind::TextArea) => Some("true".to_string()),
            _ => None,
        };
    }
    // aria-selected：Tab / TreeItem 的选中态跨节点派生（二者皆无自身选中存储）。
    // Tab 从父 TabList.selected_index 序号比；TreeItem 从所属 Tree.selected 的 NodeId
    // 恒等比。本分支必须在下面 `let cs = ...` 之前（Tab/leaf TreeItem 无 cs）。
    // 其它 kind：aria-selected 无语义（返 None）。
    if aria == "selected" {
        let node = scene.get(id)?;
        if node.kind == NodeKind::TreeItem {
            return crate::scene::control::tree_item_selected(scene, id).map(|b| b.to_string());
        }
        if node.kind != NodeKind::Tab {
            return None;
        }
        // 向上走父链找最近 TabList 祖先（Tab 的语义父）。无 → aria-selected 对此 Tab 无语义。
        let mut tablist_id = None;
        let mut cur = node.parent;
        while let Some(p) = cur {
            match scene.get(p) {
                Some(pn) if pn.kind == NodeKind::TabList => {
                    tablist_id = Some(p);
                    break;
                }
                Some(pn) => cur = pn.parent,
                None => break,
            }
        }
        let tablist_id = tablist_id?;
        let selected_index = match scene.controls.get(tablist_id) {
            Some(ControlState::TabList { selected_index, .. }) => *selected_index,
            _ => return None,
        };
        // 本 Tab 在父 TabList 的 role=tab 子里的 0 基序号（与 selected_index 同尺度）。
        // 只数 role=tab 子，忽略非 tab 中间结构（如 label 包裹），保持与解析一致。
        let my_index = scene
            .get(tablist_id)?
            .children
            .iter()
            .filter(|&&c| scene.roles.role_of(c) == Some(ROLE_TAB))
            .position(|&c| c == id)?;
        return Some((my_index == selected_index).to_string());
    }
    // aria-level（#8）：TreeItem 的层级（ARIA：顶层=1）。结构派生（嵌套深度即层级），
    // 不依赖 ControlState（leaf 无态），故在 `let cs` 之前。
    if aria == "level" {
        return crate::scene::control::tree_item_level(scene, id).map(|l| l.to_string());
    }
    let cs = scene.controls.get(id)?;
    Some(match (aria, cs) {
        ("checked", ControlState::Toggle { checked } | ControlState::Radio { checked, .. }) => {
            checked.to_string()
        }
        ("expanded", ControlState::Dropdown { open, .. }) => open.to_string(),
        // Tree branch 条目的展开态（#8）：作者用 [aria-expanded="true"] 折叠箭头旋转/
        // 高亮等定样式。leaf 虽持 TreeItem 态（指针激活目标解析用），expanded 合成仅对
        // branch 有语义——叶子不 synth（ARIA：aria-expanded 不上 leaf；web 预览模拟器
        // 同口径，只给 branch 盖戳）。
        ("expanded", ControlState::TreeItem { expanded })
            if crate::scene::control::is_branch(scene, id) =>
        {
            expanded.to_string()
        }
        ("valuenow", ControlState::Progress { value, .. } | ControlState::Slider { value, .. }) => {
            // f32 Display 用最短往返表示（50.0→"50"、33.5→"33.5"）。CSS 作者按量化后的值写选择器。
            value.to_string()
        }
        ("valuenow", ControlState::NumberField { edit, .. }) => edit.value.clone(),
        ("valuemin", ControlState::Progress { min, .. } | ControlState::Slider { min, .. }) => {
            min.to_string()
        }
        // indeterminate 的入口 = 打包期 aria-valuenow 缺席（ARIA 规范语义）；运行时
        // C# set_is_indeterminate 后由此合成镜像，作者用 [aria-indeterminate="true"] 定样式。
        ("indeterminate", ControlState::Progress { indeterminate, .. }) => {
            indeterminate.to_string()
        }
        _ => return None,
    })
}

/// 判定 compound 是否匹配 node + 状态门。
///
/// 状态门：伪类（hovered / active / disabled / focused）。
/// 通过后调 compound_matches_node 做字面匹配（tag/classes/id_attr + :nth-child 结构位置）。
fn compound_matches_with_state(c: &Compound, node_id: NodeId, scene: &Scene) -> bool {
    let node = scene.get_live(node_id, "dynamic/compound_matches_with_state");
    if c.pseudo_hover && !node.interaction.flags.contains(NodeFlags::HOVERED) {
        return false;
    }
    if c.pseudo_active && !node.interaction.flags.contains(NodeFlags::ACTIVE) {
        return false;
    }
    if c.pseudo_disabled && !node.interaction.flags.contains(NodeFlags::DISABLED) {
        return false;
    }
    if c.pseudo_focus && !node.interaction.flags.contains(NodeFlags::FOCUSED) {
        return false;
    }
    compound_matches_node(c, node_id, scene)
}

/// 完整后代链匹配（从右往左，复用 selector.rs `matches` 算法，消费 Node/Scene）。
/// 最后一个 compound 必须命中目标 node 本身（含状态门）；前面按 combinator 沿
/// parent 链找（Child=直接父，Descendant=任一祖先，带回溯）。
/// 匹配选择器 `sel` 到节点 `node_id`。`scope_bound` = 规则所属作用域根（NodeId::INVALID=全局，无边界）。
/// 后代/子代选择器沿祖先链匹配时，不穿透 scope_bound（其父在作用域外）。
pub fn match_element_with_state(
    sel: &ParsedSelector,
    node_id: NodeId,
    scene: &Scene,
    scope_bound: NodeId,
) -> bool {
    let comps = &sel.compound;
    if comps.is_empty() {
        return false;
    }
    let last = &comps[comps.len() - 1];

    // ::part(name) 臂（#57）：last = `prefix::part(name)`——其余字段匹配 **host**，
    // part 匹配目标节点。语义：目标带 part="name"（RoleInfo.attrs，attrs β 仓）∧
    // 目标的最近作用域根是 CustomElement host ∧ host 匹配 last 其余字段。嵌套组件
    // 内部节点的最近作用域根是嵌套 host——若嵌套 host 不匹配 prefix 则天然排除
    // （web ::part 不递归进嵌套 shadow 的等价物）。更左 compounds 沿 host 的祖先
    // 链常规匹配（scope_bound 照规则本体——prefix 描述规则所属作用域内的页面侧）。
    if let Some(part_name) = &last.part {
        let has_part = scene
            .roles
            .get(node_id)
            .and_then(|i| i.attr("part"))
            .is_some_and(|v| v == part_name.as_str());
        if !has_part {
            return false;
        }
        let Some(host) = nearest_scope_root_above(scene, node_id) else {
            return false; // 无作用域根 = 页面顶层，part 属性在 ::part 语义外
        };
        if scene.get_live(host, "dynamic/part:host").kind != NodeKind::CustomElement {
            return false; // List slot 根等非组件作用域不参与 ::part
        }
        if !compound_matches_with_state(last, host, scene) {
            return false;
        }
        if comps.len() == 1 {
            return true; // 裸 ::part(name)（无 host 定位条件）：组件内同名 part 即命中
        }
        return match_chain_with_state(comps, comps.len() - 1, host, scene, scope_bound);
    }

    if !compound_matches_with_state(last, node_id, scene) {
        return false;
    }
    if comps.len() == 1 {
        return true;
    }
    match_chain_with_state(comps, comps.len() - 1, node_id, scene, scope_bound)
}

/// 目标节点严格上方最近的 SCOPE_ROOT（不含自身——host 不可能是自己 part 的目标）。
/// ::part 臂用：组件展开域的 host 即此节点（页面级节点走到底返 None）。
fn nearest_scope_root_above(scene: &Scene, node: NodeId) -> Option<NodeId> {
    let mut cur = scene.get(node)?.parent;
    while let Some(nid) = cur {
        if let Some(n) = scene.get(nid) {
            if n.interaction.flags.contains(NodeFlags::SCOPE_ROOT) {
                return Some(nid);
            }
            cur = n.parent;
        } else {
            break;
        }
    }
    None
}

/// 递归匹配 comps[0..end_idx] 在 start_node 的祖先链上（同 selector.rs
/// `match_compound_chain`）。`start_node` 是已匹配 comps[end_idx] 的节点，
/// 为 comps[end_idx - 1] 找祖先（Child：直接父；Descendant：任一祖先+回溯）。
/// `scope_bound` 约束祖先链不穿透作用域根（其父在作用域外）。
fn match_chain_with_state(
    comps: &[Compound],
    end_idx: usize,
    start_node: NodeId,
    scene: &Scene,
    scope_bound: NodeId,
) -> bool {
    if end_idx == 0 {
        return true;
    }
    let target_comp = &comps[end_idx - 1];
    let combinator = comps[end_idx].combinator;
    match combinator {
        Combinator::Child => match parent_in_scope(scene, start_node, scope_bound) {
            Some(parent) => {
                compound_matches_with_state(target_comp, parent, scene)
                    && match_chain_with_state(comps, end_idx - 1, parent, scene, scope_bound)
            }
            None => false,
        },
        Combinator::Descendant => {
            let mut cur = parent_in_scope(scene, start_node, scope_bound);
            while let Some(ancestor) = cur {
                if compound_matches_with_state(target_comp, ancestor, scene)
                    && match_chain_with_state(comps, end_idx - 1, ancestor, scene, scope_bound)
                {
                    return true;
                }
                // 此祖先匹配但更左链匹配不上 → 继续往上找
                cur = parent_in_scope(scene, ancestor, scope_bound);
            }
            false
        }
    }
}

/// 取 `node` 的父，但不超过 `scope_bound`：若 node == scope_bound，其父在作用域外 → None。
/// `scope_bound == NodeId::INVALID` = 全局规则，无边界 → 直接返父（可能跨作用域）。
/// 作用域根节点的父在作用域外，后代/子代选择器不应据它匹配（不穿透边界）。
fn parent_in_scope(scene: &Scene, node: NodeId, scope_bound: NodeId) -> Option<NodeId> {
    if scope_bound != NodeId::INVALID && node == scope_bound {
        return None;
    }
    scene.get(node).and_then(|n| n.parent)
}

/// 计算每节点的所属作用域根（沿父链最近的 SCOPE_ROOT，含自身）。无作用域根祖先 → INVALID。
/// 每帧 rematch 调一次，O(节点 × 深度)；scope 校验快路径用此表 O(1) 查。
/// 根节点通常由 create_root/instantiate 打 SCOPE_ROOT，故多数节点能命中某作用域根。
/// 例外：组件展开域 host（SCOPE_ROOT + HOST_IN_PARENT_SCOPE）对自己的边界不生效——
/// 起步就跳到父节点（host 归外层页面作用域，页面规则可样式化 host 本体；后代不受影响，
/// 它们沿父链首个命中的仍是 host）。
fn compute_scope_map(scene: &Scene, node_ids: &[NodeId]) -> HashMap<NodeId, NodeId> {
    let mut map = HashMap::with_capacity(node_ids.len());
    for &id in node_ids {
        // 起始节点自身是 host → 自己的 SCOPE_ROOT 不算，从父链续走。
        let start = match scene.get(id) {
            Some(n)
                if n.interaction
                    .flags
                    .contains(NodeFlags::SCOPE_ROOT | NodeFlags::HOST_IN_PARENT_SCOPE) =>
            {
                n.parent
            }
            _ => Some(id),
        };
        let mut cur = start;
        let mut found = NodeId::INVALID;
        while let Some(nid) = cur {
            if let Some(n) = scene.get(nid) {
                if n.interaction.flags.contains(NodeFlags::SCOPE_ROOT) {
                    found = nid;
                    break;
                }
                cur = n.parent;
            } else {
                break;
            }
        }
        map.insert(id, found);
    }
    map
}

/// 全量节点重匹配（仅动态规则子集）。每节点从 base_style 重起，
/// 收集命中的动态规则（match_element_with_state），按 specificity 升序排
/// （高 specificity 后 apply 胜出）→ apply_decl 叠加 → 写 Node.style。
/// solve 每帧全量，无需 dirty 驱动。
///
/// **transition 请求发射**：对已 cascade 过（cascaded_once=true）且声明了 transition 的节点，
/// 比较旧/新级联值的可动画通道（BgColor/TextColor/Opacity）；变化则推入
/// `scene.pending_transitions`，供 Stage tick drain 后 kill 旧 tween + 提交新 tween。
/// 首次 cascade（cascaded_once=false）即时生效不产请求，并置 cascaded_once=true。
/// `root_size`/`safe_insets` 是延迟长度（视口单位/env()）的解析分母/取值源，由
/// Stage 每帧传入（viewport 概念归 Stage，不从 scene 来——见 propagate_inherited）。
///
/// **运行时规则注入（#11）**：`Scene::runtime_rule_sets`（StyleSheet.Add 通道）作为
/// 全局规则（scope_root=INVALID，跨作用域命中）追加在 pkg 规则之后收集——同
/// specificity 稳定排序下后注入赢。注入即下帧生效（本函数每帧全量）。
///
/// **var 层（#11）**：场景里存在任一 var/custom prop 源（规则声明 `--*`、值含
/// `var()`、节点带行内 deferred_inline、运行时 SetVar 条目）时走两阶段：先匹配
/// 收集（Stage 1，customs 与延迟节点登记），后 tree-order 环境 pass（Stage 2，
/// `vars::merge_env_layer` 逐节点合并环境 + 延迟声明替换重放 + finish）。无 var
/// 源的场景走单阶段原路径，零额外开销。
pub fn rematch_pseudo_classes(scene: &mut Scene, root_size: (f32, f32), safe_insets: [f32; 4]) {
    // 预提取 specificity 元组 + owned rule 副本 + scope_root（避免借 scene.dynamic_rules 跨 get_mut）。
    // pkg scoped 规则在前；运行时注入集在后（全局语义 + 同 specificity 后注入赢）。
    let mut rules_with_spec: Vec<(u32, u32, u32, DynamicRule, NodeId)> = scene
        .dynamic_rules
        .entries
        .iter()
        .map(|sr| {
            (
                sr.rule.selector.specificity.0,
                sr.rule.selector.specificity.1,
                sr.rule.selector.specificity.2,
                sr.rule.clone(),
                sr.scope_root,
            )
        })
        .collect();
    rules_with_spec.extend(scene.runtime_rule_sets.iter().flat_map(|rs| {
        rs.rules.iter().map(|r| {
            (
                r.selector.specificity.0,
                r.selector.specificity.1,
                r.selector.specificity.2,
                r.clone(),
                NodeId::INVALID,
            )
        })
    }));
    // var 层开关：任一规则声明 --* / 任一规则值含 var( / 有 SetVar 条目 / 有节点带
    // 行内 deferred_inline。节点扫描 O(n) early-exit（Vec is_empty 判定廉价），与每帧
    // 全量 rematch 的成本模型同阶。
    let var_layer_on = !scene.node_vars.is_empty()
        || rules_with_spec.iter().any(|(_, _, _, r, _)| {
            r.declarations
                .iter()
                .any(|d| d.prop.starts_with("--") || crate::style::vars::value_has_var(&d.value))
        })
        || scene
            .nodes
            .values()
            .any(|n| !n.base_style.deferred_inline.is_empty());
    // 收集所有 NodeId（slotmap 分配，不能手造 NodeId(i)）。
    let node_ids: Vec<NodeId> = scene.nodes.values().map(|n| n.id).collect();
    // 每节点的所属作用域根（沿父链最近的 SCOPE_ROOT，含自身）。全局规则（scope_root=INVALID）
    // 跳过此过滤；scoped 规则只匹配 node_scope == rule.scope_root 的节点。
    let scope_map = compute_scope_map(scene, &node_ids);
    // set-ness：每节点显式声明了哪些可继承属性。cascade 期收集，继承 pass 消费。
    let mut set_map: HashMap<NodeId, InheritedSet> = HashMap::new();
    // var 层中间态：命中的 --* 声明（环境 pass 用）+ 延迟重放节点（匹配到 var() 值
    // 或自身带行内 var 消费声明——整段应用推迟到环境解析后，保持节点内声明序）。
    let mut customs_map: HashMap<NodeId, Vec<(String, String)>> = HashMap::new();
    let mut deferred_map: HashMap<NodeId, Vec<Declaration>> = HashMap::new();
    for node_id in node_ids {
        let mut new_style = scene
            .get_live(node_id, "dynamic/rematch:base_style")
            .base_style
            .clone();
        let node_scope = scope_map.get(&node_id).copied().unwrap_or(NodeId::INVALID);
        let mut matched: Vec<(u32, u32, u32, DynamicRule)> = Vec::new();
        for r in &rules_with_spec {
            let scope_root = r.4;
            // ::part 规则（#57）豁免作用域等值墙：目标节点在组件展开域内（node_scope =
            // host ≠ 规则的页面作用域根），穿墙许可由选择器语法显式声明——这正是
            // ::part 的存在意义（打包期页面规则穿组件内容墙的唯一通道）。匹配语义
            // （一层、host 定位、嵌套不递归）在 match_element_with_state 的 part 臂。
            let pierces_wall =
                r.3.selector
                    .compound
                    .last()
                    .is_some_and(|c| c.part.is_some());
            if scope_root != NodeId::INVALID && scope_root != node_scope && !pierces_wall {
                continue;
            }
            if match_element_with_state(&r.3.selector, node_id, scene, scope_root) {
                matched.push((r.0, r.1, r.2, r.3.clone()));
            }
        }
        // specificity 升序（高 specificity 后 apply 胜出）；同级保持原序（stable sort）
        matched.sort_by_key(|r| (r.0, r.1, r.2));
        // customs 收集 + 延迟判定（var 层开时单遍扫描匹配件）。
        let mut customs: Vec<(String, String)> = Vec::new();
        let mut has_var_decl = false;
        if var_layer_on {
            for (_, _, _, r) in &matched {
                for decl in &r.declarations {
                    if decl.prop.starts_with("--") {
                        customs.push((decl.prop.clone(), decl.value.clone()));
                    } else if crate::style::vars::value_has_var(&decl.value) {
                        has_var_decl = true;
                    }
                }
            }
        }
        // 行内 var 消费声明（base_style.deferred_inline 的非 --* 条目）也要求延迟——
        // 它们的值要等环境解析（无论规则是否命中）。
        let has_inline_var = var_layer_on
            && scene
                .get_live(node_id, "dynamic/rematch:deferred_inline")
                .base_style
                .deferred_inline
                .iter()
                .any(|d| !d.prop.starts_with("--"));
        if !customs.is_empty() {
            customs_map.insert(node_id, customs);
        }
        if var_layer_on && (has_var_decl || has_inline_var) {
            // 延迟节点：typed 规则声明（--* 除外）按匹配序登记，环境 pass 统一重放。
            // old_style/cascaded_once 不登记者——finish 时从节点读（frame 内 style 只在
            // finish 写，届时仍是旧值）。
            let decls: Vec<Declaration> = matched
                .iter()
                .flat_map(|(_, _, _, r)| r.declarations.iter())
                .filter(|d| !d.prop.starts_with("--"))
                .cloned()
                .collect();
            deferred_map.insert(node_id, decls);
            continue;
        }
        // apply 声明并收集 set-ness：apply_decl 返回 true = 该 prop 成功写入
        // → 若是可继承属性，记对应 bit，供继承 pass 判"子是否显式声明"。
        // Seed from base_style.inherited_set (package-time baked declarations),
        // then OR dynamic cascade bits on top.
        // new_style is a fresh clone of base_style (not yet modified by apply_decl below),
        // so its inherited_set == base_style.inherited_set at this point.
        let mut inh: InheritedSet = new_style.inherited_set;
        // display:flex 声明在场标记：span 等行内元素的打包期 display 本就是 Flex
        //（inline→flex hack），按值差判不了「作者显式要 flex」——只有声明本身是证据。
        // 级联终态仍为 Flex 时翻转布局策略（rich_text_block 折叠 → flex 容器）：
        // slot 投射内容在页面宇宙分类，看不到组件 `<style>` 的 display:flex，
        // 策略切换必须发生在运行时 cascade（架构不变量：display 选择布局 Strategy）。
        let mut display_decl_seen = false;
        for (_, _, _, r) in &matched {
            for decl in &r.declarations {
                // --* 声明不是 typed 声明（var 层已收集；var 层关时规则里不可能有——gate 拒）
                if var_layer_on && decl.prop.starts_with("--") {
                    continue;
                }
                // CSS inline > class：base_style.inline_declared 标记的属性由打包期 inline
                // style 声明，class 规则不覆盖（如 dialog-overlay 的 inline display:none 不被
                // .dialog-overlay{display:flex} 覆盖）。inline_bit 返 None 的属性（transition 等）
                // 无 inline 来源，照常应用。
                if let Some(bit) = inline_bit(&decl.prop) {
                    if new_style.inline_declared & bit != 0 {
                        continue;
                    }
                }
                if decl.prop == "display" {
                    display_decl_seen = true;
                }
                if apply_decl(&mut new_style, &decl.prop, &decl.value) {
                    if let Some(bit) = inherited_bit(&decl.prop) {
                        inh.0 |= bit;
                    }
                }
            }
        }
        finish_rematch_node(
            scene,
            node_id,
            new_style,
            inh,
            display_decl_seen,
            &mut set_map,
        );
    }
    // var 环境 pass（#11）：tree-order 逐节点合并环境 + 延迟声明替换重放 + finish。
    if var_layer_on {
        var_env_pass(scene, &customs_map, &deferred_map, &mut set_map);
    }
    // 通用可继承属性传播：每节点从 base_style 独立 cascade（不读父），故继承须 rematch 后
    // 按 tree order 补一次：子未显式声明（set_map 无该 bit）→ 取父 effective 值。
    propagate_inherited(scene, &set_map, root_size, safe_insets);
}

/// rematch 每节点的收尾（立即路径与环境 pass 延迟路径共用）：C# inline_override 应用
/// （最高优先级）→ set_map 登记 → transition 请求发射 → style 写回（值比较短路）→
/// touchable 同步 → rich_text_block 翻转 → CASCALED 置位。
///
/// old_style/cascaded_once 此处从节点现读（本帧 style 只在本函数写，读到的一定是
/// 旧值）——两条路径统一，免显式快照传递。
fn finish_rematch_node(
    scene: &mut Scene,
    node_id: NodeId,
    mut new_style: ResolvedStyle,
    mut inh: InheritedSet,
    display_decl_seen: bool,
    set_map: &mut HashMap<NodeId, InheritedSet>,
) {
    let (old_style, cascaded_once) = {
        let n = scene.get_live(node_id, "dynamic/rematch:old_style");
        (
            n.style.clone(),
            n.interaction.flags.contains(NodeFlags::CASCALED),
        )
    };
    // inline_override 应用（最高优先级，动态规则之后）。
    // 按 inline_set 把 inline_override 字段拷进 new_style；继承子集 OR 进 set_map，
    // 使 propagate 把含 inline 的父值传给未自设的子、且本节点自身不被父覆盖。
    // inline_set 默认空 → 对没设 inline 的节点 no-op。
    {
        let n_ref = scene.get_live(node_id, "dynamic/rematch:inline_set");
        let inline_set = n_ref.inline_set;
        if inline_set.0 != 0 {
            // 直传 &n_ref.inline_override（不可变借，new_style 是 local 不冲突；
            // block 结束 n_ref 借释放，后续 set_map.insert/get_mut 不受影响）。
            // 省 ResolvedStyle clone（含 Vec<TransitionSpec>/text_effects，每帧每 inline 节点）。
            apply_inline_override(&mut new_style, &n_ref.inline_override, inline_set);
            // 只把继承子集并进 set_map；非继承 bit 不影响 propagate。
            // InheritedSet/InlineSet 同为 u64，直接掩码无需截位。
            inh.0 |= inline_set.0 & INH_ALL_MASK;
        }
    }
    set_map.insert(node_id, inh);
    // transition 声明读自级联结果（new_style）：base/inline 烘焙经
    // base_style.clone 进入，动态 class 规则经 apply_decl 写入——两源统一。
    let transition_decl = new_style.transition.clone();
    for ts in &transition_decl {
        if cascaded_once && ts.duration > 0.0 {
            emit_transition_requests(scene, node_id, *ts, &old_style, &new_style);
        }
    }
    let node = scene.get_live_mut(node_id, "dynamic/rematch:write");
    // 值比较短路：稳态帧 style 逐字节不变——不写不 bump（bump 会打死 render build
    // 缓存）。变化才写 + render_input_version +1（A2 增量指纹的失效信号）。
    if node.style != new_style {
        node.style = new_style;
        node.render_input_version += 1;
    }
    // pointer-events（style.touchable）级联终态回写 interaction——hit_test 判据在
    // interaction（建节点时从 base_style 拷贝一次），类规则驱动的 pointer-events
    // 只落 style 层的话永远到不了命中（mini-hud 根 pointer-events:none 不吞命中的
    // 实锤）。运行时 set_node_touchable 写的是 base_style（重起源），此处随级联
    // 同步不与其冲突。inline 声明路径不经过此分叉（打包期 bake 进 base_style）。
    if node.interaction.touchable != node.style.touchable {
        node.interaction.touchable = node.style.touchable;
    }
    if display_decl_seen && node.style.display_mode == DisplayMode::Flex && node.rich_text_block {
        node.rich_text_block = false;
    }
    node.interaction.flags.insert(NodeFlags::CASCALED);
}

/// var 环境 pass（#11）：tree-order DFS。每节点环境 = 父环境 + 本层声明覆盖
/// （优先级：规则命中的 --* < 行内 deferred_inline 的 --* < 运行时 SetVar——
/// `vars::merge_env_layer` 内同层后者胜 + 胜出集递归解析）。环境就绪后重放该节点
/// 的延迟声明（规则源按匹配序替换应用 → 行内 var 消费声明按 inline 优先级应用）
/// 并 finish。customs-only 节点（无 typed 延迟）不重放只贡献环境。
fn var_env_pass(
    scene: &mut Scene,
    customs_map: &HashMap<NodeId, Vec<(String, String)>>,
    deferred_map: &HashMap<NodeId, Vec<Declaration>>,
    set_map: &mut HashMap<NodeId, InheritedSet>,
) {
    use std::collections::HashSet;

    /// 环境解析失败（环/链断）的 warn-once（scene.warnings + warned_keys 去重，
    /// 与 list.rs 运行时警告同一机制）。
    fn warn_var_once(scene: &mut Scene, msg: &str) {
        if scene.warned_keys.insert(format!("var:{msg}")) {
            scene.warnings.push(msg.to_string());
        }
    }

    fn var_env_rec(
        scene: &mut Scene,
        id: NodeId,
        parent_env: &crate::style::vars::VarEnv,
        customs_map: &HashMap<NodeId, Vec<(String, String)>>,
        deferred_map: &HashMap<NodeId, Vec<Declaration>>,
        set_map: &mut HashMap<NodeId, InheritedSet>,
        finished: &mut HashSet<NodeId>,
    ) {
        // 本层声明（优先级序喂入：规则 → 行内 --* → SetVar，同层后者胜）。
        let mut layer: Vec<(String, String)> = Vec::new();
        if let Some(c) = customs_map.get(&id) {
            layer.extend(c.iter().cloned());
        }
        {
            let n = scene.get_live(id, "dynamic/var_env_pass:layer");
            for d in &n.base_style.deferred_inline {
                if d.prop.starts_with("--") {
                    layer.push((d.prop.clone(), d.value.clone()));
                }
            }
        }
        if let Some(vars) = scene.node_vars.get(&id) {
            layer.extend(vars.iter().cloned());
        }
        // 环境合并（层空 → 复用父引用，零分配）。合并期 on_invalid 走 warn-once。
        let merged;
        let env: &crate::style::vars::VarEnv = if layer.is_empty() {
            parent_env
        } else {
            let mut sink: Vec<String> = Vec::new();
            merged = crate::style::vars::merge_env_layer(parent_env, &layer, &mut |m| {
                sink.push(m.to_string())
            });
            for m in sink {
                warn_var_once(scene, &m);
            }
            &merged
        };
        // 延迟节点重放 + finish（old/cascaded 由 finish 现读）。
        if deferred_map.contains_key(&id) {
            replay_deferred_node(scene, id, env, deferred_map, set_map);
            finished.insert(id);
        }
        let children = scene
            .get_live(id, "dynamic/var_env_pass:children")
            .children
            .clone();
        for c in children {
            var_env_rec(scene, c, env, customs_map, deferred_map, set_map, finished);
        }
    }

    let roots = scene.roots.clone();
    let mut finished: HashSet<NodeId> = HashSet::new();
    let empty = crate::style::vars::VarEnv::new();
    for root in roots {
        var_env_rec(
            scene,
            root,
            &empty,
            customs_map,
            deferred_map,
            set_map,
            &mut finished,
        );
    }
    // 防御：不在任何 root 树上的延迟节点（游离克隆等手术中间态）——空父环境收尾，
    // 不留未 CASCADED 节点（customs 只来自本层，环境仍可用）。
    let strays: Vec<NodeId> = deferred_map
        .keys()
        .copied()
        .filter(|id| !finished.contains(id))
        .collect();
    for id in strays {
        replay_deferred_node(scene, id, &empty, deferred_map, set_map);
    }
}

/// 延迟节点的声明重放（var 环境 pass 内，#11）：new_style 从 base 重起 →
/// 规则源延迟声明按匹配序替换应用（inline_declared skip 重查、替换失败 warn-once
/// 跳过）→ 行内 var 消费声明（deferred_inline 非 --*）按 inline 优先级应用 →
/// finish_rematch_node 收尾。
fn replay_deferred_node(
    scene: &mut Scene,
    id: NodeId,
    env: &crate::style::vars::VarEnv,
    deferred_map: &HashMap<NodeId, Vec<Declaration>>,
    set_map: &mut HashMap<NodeId, InheritedSet>,
) {
    use crate::style::vars::substitute_value;
    let mut new_style = scene
        .get_live(id, "dynamic/replay_deferred:base")
        .base_style
        .clone();
    let mut inh: InheritedSet = new_style.inherited_set;
    let mut display_decl_seen = false;
    if let Some(decls) = deferred_map.get(&id) {
        for decl in decls {
            // CSS inline > class（与立即路径同一 skip 语义）。
            if let Some(bit) = inline_bit(&decl.prop) {
                if new_style.inline_declared & bit != 0 {
                    continue;
                }
            }
            // 替换（失败 = 缺失/环且无 fallback → 声明 invalid 跳过，warn-once）。
            let value = match substitute_value(&decl.value, env) {
                Some(v) => v,
                None => {
                    let msg = format!(
                        "declaration \"{}: {}\" skipped — var() did not resolve (missing \
                         custom property or cycle without fallback)",
                        decl.prop, decl.value
                    );
                    if scene.warned_keys.insert(format!("var:{msg}")) {
                        scene.warnings.push(msg);
                    }
                    continue;
                }
            };
            if decl.prop == "display" {
                display_decl_seen = true;
            }
            if apply_decl(&mut new_style, &decl.prop, &value) {
                if let Some(bit) = inherited_bit(&decl.prop) {
                    inh.0 |= bit;
                }
            }
        }
    }
    // 行内 var 消费声明（deferred_inline 的非 --* 条目）：inline 优先级——不查
    // inline_declared skip（它们就是 inline 源本身），在规则声明之后应用。
    {
        let decls = scene
            .get_live(id, "dynamic/replay_deferred:inline")
            .base_style
            .deferred_inline
            .clone();
        for decl in &decls {
            if decl.prop.starts_with("--") {
                continue;
            }
            let value = match substitute_value(&decl.value, env) {
                Some(v) => v,
                None => {
                    let msg = format!(
                        "declaration \"{}: {}\" skipped — var() did not resolve (missing \
                         custom property or cycle without fallback)",
                        decl.prop, decl.value
                    );
                    if scene.warned_keys.insert(format!("var:{msg}")) {
                        scene.warnings.push(msg);
                    }
                    continue;
                }
            };
            if decl.prop == "display" {
                display_decl_seen = true;
            }
            if apply_decl(&mut new_style, &decl.prop, &value) {
                if let Some(bit) = inherited_bit(&decl.prop) {
                    inh.0 |= bit;
                }
            }
        }
    }
    finish_rematch_node(scene, id, new_style, inh, display_decl_seen, set_map);
}

/// 按 set 位图把 `inline_override` 字段拷进 style（最高优先级覆盖）。覆盖全部 11 个继承
/// 字段（INH_*，bits 0-7 + 33-35）+ 非继承字段（INLINE_*，bits 8-32 及 36/37，z-index 在
/// bit 32、text-decoration 在 bit 36、cursor 在 bit 37）。
/// INLINE_DISPLAY 一对应两字段（`taffy_style.display` + `display_mode`，与 apply_decl
/// 行为对齐），其余 INLINE_* 一对一映射到 ResolvedStyle/taffy_style 字段。
///
/// 该函数不改 `style.inherited_set`——inline 的继承子集由调用方 OR 进 set_map。
fn apply_inline_override(style: &mut ResolvedStyle, inline: &ResolvedStyle, set: InlineSet) {
    let s = set.0;
    // 单字段拷贝：`$($f:ident).+` 支持顶层（color）+ taffy 嵌套（taffy_style.size.width）路径。
    macro_rules! cpy {
        ($($f:ident).+, $bit:expr) => {
            if s & ($bit) != 0 {
                style.$($f).+ = inline.$($f).+.clone();
            }
        };
    }
    // 字段 + 延迟长度槽成对拷（宽高/padding/margin/gap 等几何通道既有 taffy 值也有
    // viewport 平行槽）：只拷 taffy 值会让 solve 期 viewport.apply 用旧槽覆写 inline。
    macro_rules! cpy_slot {
        ($($f:ident).+, $($vf:ident).+, $bit:expr) => {
            if s & ($bit) != 0 {
                style.$($f).+ = inline.$($f).+.clone();
                style.viewport.$($vf).+ = inline.viewport.$($vf).+;
            }
        };
    }
    // 继承属性（bits 0-7 + 33-35）
    cpy_slot!(font_size, font_size, INH_FONT_SIZE);
    cpy!(color, INH_COLOR);
    cpy!(font_family, INH_FONT_FAMILY);
    cpy!(font_weight, INH_FONT_WEIGHT);
    cpy!(text_align, INH_TEXT_ALIGN);
    // line-height 双槽（倍数/px）同一 bit：声明的两形一起保、继承的两形一起拷。
    cpy!(line_height, INH_LINE_HEIGHT);
    cpy!(line_height_px, INH_LINE_HEIGHT);
    cpy_slot!(letter_spacing, letter_spacing, INH_LETTER_SPACING);
    cpy!(white_space, INH_WHITE_SPACE);
    cpy!(overflow_wrap, INH_OVERFLOW_WRAP);
    cpy!(word_break, INH_WORD_BREAK);
    cpy!(text_wrap, INH_TEXT_WRAP);
    // 非继承属性（bits 8-31）——taffy_style 子字段 + viewport 平行槽
    cpy_slot!(taffy_style.size.width, width, INLINE_WIDTH);
    cpy_slot!(taffy_style.size.height, height, INLINE_HEIGHT);
    cpy_slot!(taffy_style.min_size.width, min_width, INLINE_MIN_WIDTH);
    cpy_slot!(taffy_style.min_size.height, min_height, INLINE_MIN_HEIGHT);
    cpy_slot!(taffy_style.max_size.width, max_width, INLINE_MAX_WIDTH);
    cpy_slot!(taffy_style.max_size.height, max_height, INLINE_MAX_HEIGHT);
    cpy_slot!(taffy_style.padding, padding, INLINE_PADDING);
    cpy_slot!(taffy_style.margin, margin, INLINE_MARGIN);
    cpy!(taffy_style.border, INLINE_BORDER_WIDTH);
    if s & INLINE_GAP != 0 {
        style.taffy_style.gap = inline.taffy_style.gap;
        style.viewport.row_gap = inline.viewport.row_gap;
        style.viewport.column_gap = inline.viewport.column_gap;
    }
    cpy!(taffy_style.flex_direction, INLINE_FLEX_DIRECTION);
    cpy!(taffy_style.flex_wrap, INLINE_FLEX_WRAP);
    cpy!(taffy_style.justify_content, INLINE_JUSTIFY_CONTENT);
    cpy!(taffy_style.align_items, INLINE_ALIGN_ITEMS);
    cpy!(overflow_x, INLINE_OVERFLOW_X);
    cpy!(overflow_y, INLINE_OVERFLOW_Y);
    // INLINE_POSITION：apply_decl 同时设 taffy_style.position + position_declared，双字段覆盖。
    if s & INLINE_POSITION != 0 {
        style.taffy_style.position = inline.taffy_style.position;
        style.position_declared = inline.position_declared;
    }
    // inset 四边：taffy 值 + viewport.inset[idx]（[t,r,b,l]）成对拷，同 cpy_slot 理由。
    if s & INLINE_TOP != 0 {
        style.taffy_style.inset.top = inline.taffy_style.inset.top;
        style.viewport.inset[0] = inline.viewport.inset[0];
    }
    if s & INLINE_RIGHT != 0 {
        style.taffy_style.inset.right = inline.taffy_style.inset.right;
        style.viewport.inset[1] = inline.viewport.inset[1];
    }
    if s & INLINE_BOTTOM != 0 {
        style.taffy_style.inset.bottom = inline.taffy_style.inset.bottom;
        style.viewport.inset[2] = inline.viewport.inset[2];
    }
    if s & INLINE_LEFT != 0 {
        style.taffy_style.inset.left = inline.taffy_style.inset.left;
        style.viewport.inset[3] = inline.viewport.inset[3];
    }
    // 非继承属性——视觉/渲染字段
    cpy!(background_color, INLINE_BACKGROUND_COLOR);
    cpy!(opacity, INLINE_OPACITY);
    // INLINE_Z_INDEX：apply_decl 同时设 z_index + z_declared（stacking 分类的
    // 组成对，同 INLINE_POSITION 双字段先例）——单拷 z_index 会让回退时 z_declared
    // 残留、stacking::classify 把已回退 static 的节点错判成 stacking context。
    if s & INLINE_Z_INDEX != 0 {
        style.z_index = inline.z_index;
        style.z_declared = inline.z_declared;
    }
    cpy!(text_decoration, INLINE_TEXT_DECORATION);
    cpy!(cursor, INLINE_CURSOR);
    // INLINE_DISPLAY：apply_decl 同时设 taffy_style.display + display_mode，需双字段覆盖。
    if s & INLINE_DISPLAY != 0 {
        style.taffy_style.display = inline.taffy_style.display;
        style.display_mode = inline.display_mode;
    }
}

/// 通用可继承属性传播（tree-order DFS）。子未显式声明的可继承字段 → 取父 effective 值。
/// `effective` = 节点当前 style 值（anim override 本轮仅 color 用过，font 等无 anim）。
///
/// 延迟长度（视口单位/env()）解析嵌在本走查：font-size/letter-spacing 是继承属性，
/// 必须父先解析成 px、子再拷贝（px 字段是传播载体）；border-radius 非继承但同走查
/// 顺带落 px 缓存。几何通道（padding/gap 等）不走这里——solve 建树时 viewport.apply
/// 直接覆写 taffy 副本。
fn propagate_inherited(
    scene: &mut Scene,
    set_map: &HashMap<NodeId, InheritedSet>,
    root_size: (f32, f32),
    safe_insets: [f32; 4],
) {
    let roots = scene.roots.clone();
    for root in roots {
        propagate_inherited_rec(scene, root, None, set_map, root_size, safe_insets);
    }
}

#[allow(clippy::too_many_arguments)]
fn propagate_inherited_rec(
    scene: &mut Scene,
    id: NodeId,
    parent_style: Option<ResolvedStyle>,
    set_map: &HashMap<NodeId, InheritedSet>,
    root_size: (f32, f32),
    safe_insets: [f32; 4],
) {
    let (mut my_style, children) = {
        let n = scene.get_live(id, "dynamic/propagate_inherited:read");
        (n.style.clone(), n.children.clone())
    };
    // 本节点延迟长度先解析（ViewportStyle 是 Copy，拷出再借 &mut 消字段/整体双借）
    let vp = my_style.viewport;
    vp.apply_visual(&mut my_style, root_size, safe_insets);
    // 父 effective = 父传下来的 style 快照（已含父自己的继承结果，因 tree order）
    if let Some(parent_eff) = parent_style {
        let inh = set_map.get(&id).copied().unwrap_or_default();
        let mut new_style = my_style;
        macro_rules! copy_if_unset {
            ($field:ident, $bit:expr) => {
                if (inh.0 & $bit) == 0 {
                    new_style.$field = parent_eff.$field;
                }
            };
        }
        copy_if_unset!(font_size, INH_FONT_SIZE);
        // anim text-color override to children is dropped (the old propagate_color_inheritance
        // had this); restore when text anim + inheritance interact.
        copy_if_unset!(color, INH_COLOR);
        copy_if_unset!(font_family, INH_FONT_FAMILY);
        copy_if_unset!(font_weight, INH_FONT_WEIGHT);
        copy_if_unset!(text_align, INH_TEXT_ALIGN);
        copy_if_unset!(line_height, INH_LINE_HEIGHT);
        copy_if_unset!(line_height_px, INH_LINE_HEIGHT);
        copy_if_unset!(letter_spacing, INH_LETTER_SPACING);
        copy_if_unset!(white_space, INH_WHITE_SPACE);
        copy_if_unset!(overflow_wrap, INH_OVERFLOW_WRAP);
        copy_if_unset!(word_break, INH_WORD_BREAK);
        copy_if_unset!(text_wrap, INH_TEXT_WRAP);
        // per-clone，节点多时换就地改 + 父快照
        let eff_for_children = new_style.clone();
        let node = scene.get_live_mut(id, "dynamic/propagate_inherited:write");
        if node.style != new_style {
            node.style = new_style;
            node.render_input_version += 1;
        }
        // 向下传我更新后的 style 作为子 effective
        for c in children {
            propagate_inherited_rec(
                scene,
                c,
                Some(eff_for_children.clone()),
                set_map,
                root_size,
                safe_insets,
            );
        }
    } else {
        // 根节点：无父继承，effective = 自己 style（延迟长度已解析，落盘供 solve/render 读）
        scene
            .get_live_mut(id, "dynamic/propagate_inherited:write-root")
            .style = my_style.clone();
        for c in children {
            propagate_inherited_rec(
                scene,
                c,
                Some(my_style.clone()),
                set_map,
                root_size,
                safe_insets,
            );
        }
    }
}

/// 声明式动画启停。rematch 之后、solve 之前
/// 调用：读节点 computed `style.animation`（rematch 已把动态规则的 animation 声明叠加进
/// style），与节点活跃 player 比对，只增删不推进时间轴（推进是 update_all 的事）。
///
/// 决策规则（与 `update_all` 的 Completed 保留设计衔接）：
/// - **声明出现（新 name）** → 建 player：从 `Scene.keyframes` 全局表按 name 拷 KeyframesRule；
///   fill backwards/both 立即算首帧写 NodeAnim（防 delay 期闪 base）；
///   `animation-play-state: paused` 声明的建 Paused player。未知 name → 跳过（CSS 语义=无动画）。
/// - **声明消失** → 回收 player（含 fill forwards/both 的 Completed player）：
///   清它持有的通道回 None → tween/base 接管。同节点多 player 共享通道时，只清"移除者持有
///   且无剩余 player 持有"的通道（防误清仍在播的动画）。
/// - **同名参数变**（duration/delay/iteration/fill 等）→ kill 旧 + 建新（合法重播）。
/// - **同名已存在（含 Completed）** → 不重播（防 fill none 的结束标记被无限重启）。
/// - **`programmatic` player**（node.Play 建）→ 完全跳过：不回收、不算已存在
///   （同名声明出现时另建 class player，二者独立）。
///
/// 多 animation（`animation: a .3s, b .5s`）：每条声明独立建 player；update_all 按插入序
/// 写，后声明覆盖同通道。
pub fn sync_animation_players(scene: &mut Scene) {
    use crate::scene::animation::{
        clear_channels, owned_channels, write_frame, KeyframePlayer, PlayerKey, PlayerPlayState,
    };
    use crate::style::resolved::{AnimationFillMode, AnimationPlayState};
    use std::collections::HashSet;

    // 按节点分组现有 player（slotmap 迭代 + 克隆 spec，避免跨可变借）。
    // 悬空节点（节点已删，update_all 同款双保险）：直接回收，不进分组。
    let mut node_players: HashMap<NodeId, Vec<(PlayerKey, AnimationSpec, bool)>> = HashMap::new();
    let mut dead_keys: Vec<PlayerKey> = Vec::new();
    for (k, p) in &scene.players {
        if scene.nodes.contains_key(p.node.to_key()) {
            node_players
                .entry(p.node)
                .or_default()
                .push((k, p.spec.clone(), p.programmatic));
        } else {
            dead_keys.push(k);
        }
    }

    let node_ids: Vec<NodeId> = scene.nodes.values().map(|n| n.id).collect();
    let mut remove_keys: Vec<PlayerKey> = dead_keys;
    let mut insert_specs: Vec<(NodeId, AnimationSpec)> = Vec::new();

    for node in node_ids {
        let declared = scene
            .get(node)
            .map(|n| n.style.animation.clone())
            .unwrap_or_default();
        let existing = node_players.remove(&node).unwrap_or_default();
        if declared.is_empty() && existing.is_empty() {
            continue;
        }
        // 每条声明匹配一个未占用的非 programmatic player：
        //  - 同名同参（含 Completed）→ 保留；
        //  - 同名异参 → kill 旧 + 重播（参数变 = 合法重启）；
        //  - 无同名 → 新建。
        let mut used: HashSet<PlayerKey> = HashSet::new();
        for spec in &declared {
            // 空 name = 长划先于 animation-name 的惰性声明（apply_animation_longhand
            // 创建的 initial spec）——不建 player，等 name 到位再启播。
            if spec.name.is_empty() {
                continue;
            }
            match existing
                .iter()
                .find(|(k, ps, prog)| !prog && !used.contains(k) && ps.name == spec.name)
            {
                Some((k, ps, _)) => {
                    used.insert(*k);
                    if ps != spec {
                        remove_keys.push(*k);
                        insert_specs.push((node, spec.clone()));
                    }
                }
                None => insert_specs.push((node, spec.clone())),
            }
        }
        // 声明消失的非 programmatic player（含 Completed）→ 回收。
        for (k, _, prog) in &existing {
            if !prog && !used.contains(k) {
                remove_keys.push(*k);
            }
        }
    }

    // 应用：先移除（清通道掩码 = 移除者持有 ∩ 无剩余持有，防共享通道误清），
    // 再插入（backwards/both 立即写首帧；paused 声明建 Paused player）。
    for k in &remove_keys {
        let Some(p) = scene.players.remove(*k) else {
            continue;
        };
        let own = owned_channels(&p);
        let remaining =
            scene
                .players
                .values()
                .filter(|q| q.node == p.node)
                .fold([false; 8], |acc, q| {
                    let m = owned_channels(q);
                    let mut out = acc;
                    for (o, b) in out.iter_mut().zip(m) {
                        *o |= b;
                    }
                    out
                });
        let mut clear = [false; 8];
        for (i, c) in clear.iter_mut().enumerate() {
            *c = own[i] && !remaining[i];
        }
        clear_channels(&mut scene.anim, p.node, clear);
    }
    for (node, spec) in insert_specs {
        let Some(rule) = scene.keyframes.get(&spec.name).cloned() else {
            continue; // 未知 animation-name：CSS 语义 = 无动画（打包期 validate 应已拦，防御）
        };
        let mut player = KeyframePlayer::new(node, spec.clone(), rule);
        if spec.play_state == AnimationPlayState::Paused {
            player.play_state = PlayerPlayState::Paused;
        }
        // 首帧立即写：不等下帧 update_all，防 delay 期闪 base。
        let first = player.advance(0.0);
        scene.players.insert(player);
        if matches!(
            spec.fill_mode,
            AnimationFillMode::Backwards | AnimationFillMode::Both
        ) {
            // transform translate 的 LenPct 百分比按节点布局尺寸解析（#77）。
            let size = scene
                .get(node)
                .map(|n| [n.layout_rect.w, n.layout_rect.h])
                .unwrap_or([0.0, 0.0]);
            write_frame(&mut scene.anim, node, first.props, size);
        }
    }
}

/// 比较旧/新级联值的可动画通道；变化的（且 transition 声明覆盖该通道）推入 pending_transitions。
/// `ts.prop`=None 表 all（任一通道变化都触发）；Some(p) 仅触发该通道。
/// start 取进行中 tween 的 override（mid-flight 连续，无 snap），无则取旧级联值。
fn emit_transition_requests(
    scene: &mut Scene,
    node: NodeId,
    ts: TransitionSpec,
    old: &ResolvedStyle,
    new: &ResolvedStyle,
) {
    use crate::tween::{TransitionRequest, TweenProp};
    let wants = |p: TweenProp| ts.prop.is_none() || matches!(ts.prop, Some(q) if q == p);
    let anim = scene.anim.get(node); // mid-flight override 作 start（无则 old cascade 值）
                                     // background-color: Option<[f32;4]>（None 视作透明 [0,0,0,0]）
    let pad5 = |v: [f32; 4]| [v[0], v[1], v[2], v[3], 0.0];
    if wants(TweenProp::BgColor) {
        let a = old.background_color.unwrap_or([0.0; 4]);
        let b = new.background_color.unwrap_or([0.0; 4]);
        if a != b {
            let start = anim.and_then(|x| x.bg_color).unwrap_or(a);
            scene.pending_transitions.push(TransitionRequest {
                node,
                prop: TweenProp::BgColor,
                start: pad5(start),
                end: pad5(b),
                ease: ts.ease,
                delay: ts.delay,
                duration: ts.duration,
                shadow: None,
            });
        }
    }
    // text color: [f32;4]（color 字段，非 Option）
    if wants(TweenProp::TextColor) {
        let a = old.color;
        let b = new.color;
        if a != b {
            let start = anim.and_then(|x| x.text_color).unwrap_or(a);
            scene.pending_transitions.push(TransitionRequest {
                node,
                prop: TweenProp::TextColor,
                start: pad5(start),
                end: pad5(b),
                ease: ts.ease,
                delay: ts.delay,
                duration: ts.duration,
                shadow: None,
            });
        }
    }
    // opacity: f32（标量，pack 进首分量）
    if wants(TweenProp::Opacity) && (old.opacity - new.opacity).abs() > 1e-6 {
        let start = anim.and_then(|x| x.opacity).unwrap_or(old.opacity);
        scene.pending_transitions.push(TransitionRequest {
            node,
            prop: TweenProp::Opacity,
            start: [start, 0.0, 0.0, 0.0, 0.0],
            end: [new.opacity, 0.0, 0.0, 0.0, 0.0],
            ease: ts.ease,
            delay: ts.delay,
            duration: ts.duration,
            shadow: None,
        });
    }
    // transform：整矩阵 TRS 分解 → 单复合通道插值（CSS 对结构不同的 transform 列表
    // 的 fallback 语义）。围栏子集（translate/scale/rotate 复合）恒可分解；镜像被
    // 编码为负 sy。start 取 mid-flight override 的分解（连续，无 snap）。
    if wants(TweenProp::Transform) {
        let a = &old.transform.matrix;
        let b = &new.transform.matrix;
        let changed = (0..6).any(|i| (a[i] - b[i]).abs() > 1e-6);
        if changed {
            let start = match anim.and_then(|x| x.transform) {
                Some(m) => crate::transform::decompose_trs(&m),
                None => crate::transform::decompose_trs(a),
            };
            scene.pending_transitions.push(TransitionRequest {
                node,
                prop: TweenProp::Transform,
                start,
                end: crate::transform::decompose_trs(b),
                ease: ts.ease,
                delay: ts.delay,
                duration: ts.duration,
                shadow: None,
            });
        }
    }
    // #10 layout/box-shadow 通道。width/height 端点须同域且非 auto（域判定：viewport
    // 平行槽优先——vw 声明的 taffy 槽是 length(0) 占位）；违反 → snap（不建 tween，
    // rematch 已写新值 = 直接跳变）+ 警告事件。围栏对静态可见端点硬拒，这里漏的
    // 只有运行时 add_class 组合。mid-flight start 全部先取值（anim 引用不跨任何 push）。
    let anim_width = anim.and_then(|x| x.width);
    let anim_height = anim.and_then(|x| x.height);
    let anim_flex = anim.and_then(|x| x.flex_grow);
    let anim_shadow = anim.and_then(|x| x.box_shadow.clone());
    emit_size_transition(scene, node, ts, anim_width, TweenProp::Width, old, new);
    emit_size_transition(scene, node, ts, anim_height, TweenProp::Height, old, new);
    if wants(TweenProp::FlexGrow) {
        let a = old.taffy_style.flex_grow;
        let b = new.taffy_style.flex_grow;
        if (a - b).abs() > 1e-6 {
            let start = anim_flex.unwrap_or(a);
            scene.pending_transitions.push(TransitionRequest {
                node,
                prop: TweenProp::FlexGrow,
                start: [start, 0.0, 0.0, 0.0, 0.0],
                end: [b, 0.0, 0.0, 0.0, 0.0],
                ease: ts.ease,
                delay: ts.delay,
                duration: ts.duration,
                shadow: None,
            });
        }
    }
    if wants(TweenProp::BoxShadow) && old.box_shadow != new.box_shadow {
        let start = anim_shadow.unwrap_or_else(|| old.box_shadow.clone());
        scene.pending_transitions.push(TransitionRequest {
            node,
            prop: TweenProp::BoxShadow,
            start: [0.0; 5],
            end: [0.0; 5],
            ease: ts.ease,
            delay: ts.delay,
            duration: ts.duration,
            shadow: Some(Box::new(crate::tween::ShadowPair {
                start,
                end: new.box_shadow.clone(),
            })),
        });
    }
}

/// width/height 单通道的 transition 检测。载荷 = [value, domain_code]
/// （domain_code = LenDomain 判别值；同域保证由端点检测给出，否则 snap 不建 tween）。
/// `anim_start` = mid-flight override 的 AnimLen（None = 用旧级联值起点）。
#[allow(clippy::too_many_arguments)]
fn emit_size_transition(
    scene: &mut Scene,
    node: NodeId,
    ts: crate::style::resolved::TransitionSpec,
    anim_start: Option<crate::scene::AnimLen>,
    prop: crate::tween::TweenProp,
    old: &ResolvedStyle,
    new: &ResolvedStyle,
) {
    use crate::tween::{TransitionRequest, TweenProp};
    let wants = ts.prop.is_none() || matches!(ts.prop, Some(q) if q == prop);
    if !wants {
        return;
    }
    let len_of = |style: &ResolvedStyle| -> Option<crate::scene::AnimLen> {
        let (vp, dim) = match prop {
            TweenProp::Width => (style.viewport.width, &style.taffy_style.size.width),
            _ => (style.viewport.height, &style.taffy_style.size.height),
        };
        size_anim_len(vp, dim)
    };
    let (Some(a), Some(b)) = (len_of(old), len_of(new)) else {
        // 任一端 auto / 未声明（未声明端 = 无宽度变化可动，双端齐全才有动画语义）。
        return;
    };
    if (a.value - b.value).abs() <= 1e-6 && a.domain == b.domain {
        return;
    }
    if a.domain != b.domain {
        // 跨域端点：snap + 警告（异域混合是 fence 硬拒项的运行时漏网兜底）。
        push_snap_warning(scene, node, prop);
        return;
    }
    let start = anim_start.unwrap_or(a);
    scene.pending_transitions.push(TransitionRequest {
        node,
        prop,
        start: [start.value, start.domain as u32 as f32, 0.0, 0.0, 0.0],
        end: [b.value, b.domain as u32 as f32, 0.0, 0.0, 0.0],
        ease: ts.ease,
        delay: ts.delay,
        duration: ts.duration,
        shadow: None,
    });
}

/// 跨域端点跳变警告（EVT_TRANSITION_SNAP；payload click_count = prop 判别值）。
/// 语义：新级联值已生效（直接跳变），tween 不建——作者侧日志可观测，非静默。
fn push_snap_warning(scene: &mut Scene, node: NodeId, prop: crate::tween::TweenProp) {
    scene.pending_anim_warnings.push(crate::input::EventRecord {
        node_id: node.0,
        event_type: crate::input::EVT_TRANSITION_SNAP,
        click_count: prop as u8,
        pad: [0, 0],
        touch_id: 0,
        x: 0.0,
        y: 0.0,
        dx: 0.0,
        dy: 0.0,
    });
}

/// width/height 声明值 → AnimLen（viewport 平行槽优先；px/percent 读 taffy Dimension；
/// auto → None = 不可动画端点）。taffy 0.12 Dimension 是 tagged-pointer struct 非 enum，
/// tag 判别走 CompactLength（mapping.rs parse 同款手法）。percent 从 0..1 分数还原为
/// CSS 原始数（25% ↔ 25）。
fn size_anim_len(
    vp: Option<crate::style::resolved::DeferredLength>,
    dim: &taffy::style::Dimension,
) -> Option<crate::scene::AnimLen> {
    use crate::scene::{AnimLen, LenDomain};
    if let Some(v) = vp {
        let domain = match v {
            crate::style::resolved::DeferredLength::Viewport(vl) => match vl.unit {
                crate::style::resolved::ViewportUnit::Vw => LenDomain::Vw,
                crate::style::resolved::ViewportUnit::Vh => LenDomain::Vh,
                crate::style::resolved::ViewportUnit::Vmin => LenDomain::Vmin,
                crate::style::resolved::ViewportUnit::Vmax => LenDomain::Vmax,
            },
            // env() 端点无对应动画域（safe inset 是每帧环境值非声明长度）——
            // 与 auto 端点同判：无动画（snap），不伪造成 px。
            crate::style::resolved::DeferredLength::SafeInset(_) => return None,
        };
        return Some(AnimLen {
            domain,
            value: match v {
                crate::style::resolved::DeferredLength::Viewport(vl) => vl.value,
                _ => unreachable!(),
            },
        });
    }
    if dim.is_auto() {
        return None;
    }
    let cl = dim.into_raw();
    match cl.tag() {
        taffy::style::CompactLength::LENGTH_TAG => Some(AnimLen {
            domain: LenDomain::Px,
            value: cl.value(),
        }),
        taffy::style::CompactLength::PERCENT_TAG => Some(AnimLen {
            domain: LenDomain::Pct,
            value: cl.value() * 100.0,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::{
        ControlState, EditState, Node, NodeFlags, NodeId, NodeKind, Rect, RoleInfo, Scene,
    };
    use crate::style::resolved::TransitionSpec;
    use crate::tween::{Ease, TweenProp};

    /// ::part(name) 匹配语义（#57）：`.{host}::part({part})` 只命中「host 展开子树内
    /// 带 part 属性」的目标节点——嵌套组件内部（其作用域根是嵌套 host，prefix 不匹配
    /// 则排除）与页面级裸 part 节点（作用域根非 CustomElement）都不命中；host 自身
    /// 无 part 属性也不命中。
    #[test]
    fn part_selector_matches_target_inside_matching_host_only() {
        let mk = |kind: NodeKind| {
            let mut n = Node::default();
            n.kind = kind;
            n
        };
        // 结构：page_root(SCOPE_ROOT) ├─ host(.card, CustomElement, SCOPE_ROOT)
        //                              │    ├─ title(part="title")
        //                              │    └─ nested(CustomElement, SCOPE_ROOT) ─ inner(part="title")
        //                              └─ page_bare(part="title")
        let mut page_root = mk(NodeKind::Container);
        page_root.interaction.flags.insert(NodeFlags::SCOPE_ROOT);
        let mut host = mk(NodeKind::CustomElement);
        host.classes = vec!["card".to_string()];
        host.custom_tag = Some("x-card".to_string());
        host.interaction.flags.insert(NodeFlags::SCOPE_ROOT);
        let mut nested = mk(NodeKind::CustomElement);
        nested.custom_tag = Some("x-nest".to_string());
        nested.interaction.flags.insert(NodeFlags::SCOPE_ROOT);
        let title = mk(NodeKind::Container);
        let inner = mk(NodeKind::Container);
        let page_bare = mk(NodeKind::Container);
        let mut s = Scene::from_nodes(
            vec![page_root, host, title, nested, inner, page_bare],
            vec![(0, 1), (1, 2), (1, 3), (3, 4), (0, 5)],
        );
        let ids: Vec<NodeId> = s.nodes.values().map(|n| n.id).collect();
        let page_root = ids[0];
        let host_id = ids[1];
        let title_id = ids[2];
        let inner_id = ids[4];
        let page_bare_id = ids[5];
        for (tid, part) in [
            (title_id, "title"),
            (inner_id, "title"),
            (page_bare_id, "title"),
        ] {
            s.roles.insert(
                tid,
                RoleInfo {
                    role: None,
                    slots: Default::default(),
                    attrs: vec![("part".to_string(), part.to_string())],
                },
            );
        }

        let sel = ParsedSelector {
            raw: ".card::part(title)".to_string(),
            compound: vec![Compound {
                tag: None,
                classes: vec!["card".to_string()],
                id: None,
                combinator: Combinator::Descendant,
                pseudo_hover: false,
                pseudo_active: false,
                pseudo_disabled: false,
                pseudo_focus: false,
                pseudo_nth_child: None,
                attrs: Vec::new(),
                part: Some("title".to_string()),
            }],
            specificity: Specificity(0, 2, 1),
        };
        assert!(
            match_element_with_state(&sel, title_id, &s, page_root),
            "host 展开子树内带 part 属性的目标命中"
        );
        assert!(
            !match_element_with_state(&sel, inner_id, &s, page_root),
            "嵌套组件内部不递归（其作用域根 nested host 不匹配 .card）"
        );
        assert!(
            !match_element_with_state(&sel, page_bare_id, &s, page_root),
            "页面级裸 part 节点不命中（作用域根非 CustomElement）"
        );
        assert!(
            !match_element_with_state(&sel, host_id, &s, page_root),
            "host 自身（无 part 属性）不命中"
        );

        // 换 part 名：不同名不命中
        let sel2 = ParsedSelector {
            raw: ".card::part(other)".to_string(),
            compound: vec![Compound {
                tag: None,
                classes: vec!["card".to_string()],
                id: None,
                combinator: Combinator::Descendant,
                pseudo_hover: false,
                pseudo_active: false,
                pseudo_disabled: false,
                pseudo_focus: false,
                pseudo_nth_child: None,
                attrs: Vec::new(),
                part: Some("other".to_string()),
            }],
            specificity: Specificity(0, 2, 1),
        };
        assert!(!match_element_with_state(&sel2, title_id, &s, page_root));
    }

    /// ::part 穿墙（#57）：scoped 到页面作用域根的规则经 part 豁免命中组件内部目标
    ///（rematch 全链——作用域等值过滤放行 + part 臂匹配 + 声明落到目标节点 style）。
    /// 同规则不带 part 的对照（普通类选择器）仍被墙挡——豁免只给显式 ::part 语法。
    #[test]
    fn part_rule_pierces_scope_wall_via_rematch() {
        let mk = |kind: NodeKind| {
            let mut n = Node::default();
            n.kind = kind;
            n
        };
        let mut page_root = mk(NodeKind::Container);
        page_root.interaction.flags.insert(NodeFlags::SCOPE_ROOT);
        let mut host = mk(NodeKind::CustomElement);
        host.classes = vec!["card".to_string()];
        host.custom_tag = Some("x-card".to_string());
        host.interaction.flags.insert(NodeFlags::SCOPE_ROOT);
        let title = mk(NodeKind::Container);
        let mut s = Scene::from_nodes(vec![page_root, host, title], vec![(0, 1), (1, 2)]);
        let ids: Vec<NodeId> = s.nodes.values().map(|n| n.id).collect();
        let page_root = ids[0];
        let title_id = ids[2];
        s.roles.insert(
            title_id,
            RoleInfo {
                role: None,
                slots: Default::default(),
                attrs: vec![("part".to_string(), "title".to_string())],
            },
        );

        // part 规则 scoped 到页面根：穿墙命中组件内 title
        let part_sel = ParsedSelector {
            raw: ".card::part(title)".to_string(),
            compound: vec![Compound {
                tag: None,
                classes: vec!["card".to_string()],
                id: None,
                combinator: Combinator::Descendant,
                pseudo_hover: false,
                pseudo_active: false,
                pseudo_disabled: false,
                pseudo_focus: false,
                pseudo_nth_child: None,
                attrs: Vec::new(),
                part: Some("title".to_string()),
            }],
            specificity: Specificity(0, 2, 1),
        };
        push_scoped(
            &mut s,
            page_root,
            DynamicRule {
                selector: part_sel,
                declarations: vec![Declaration {
                    prop: "background-color".to_string(),
                    value: "#c0392b".to_string(),
                }],
            },
        );
        // 对照：普通类选择器同 scope——墙仍挡（title 无 .card 类且 node_scope=host≠page_root）
        push_scoped(&mut s, page_root, rule(".card", "color", "#0000ff"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(title_id).unwrap().style.background_color,
            Some([0.7529412, 0.22352941, 0.16862746, 1.0]),
            "::part 规则穿墙命中组件内目标（#c0392b）"
        );
        assert_ne!(
            s.get(title_id).unwrap().style.color,
            [0.0, 0.0, 1.0, 1.0],
            "普通 scoped 规则仍被墙挡（豁免只给显式 ::part 语法）"
        );
    }

    /// 把规则作为全局规则（scope_root=INVALID）推进 scene——跨作用域命中。
    /// 现有单场景测试无实例化，规则走全局路径保持原有行为（node_scope=INVALID，全局规则匹配）。
    fn push_global(s: &mut Scene, r: DynamicRule) {
        s.dynamic_rules.entries.push(ScopedRule {
            rule: r,
            scope_root: NodeId::INVALID,
        });
    }

    /// 把规则作为 scoped 规则（scope_root = 指定实例根）推进 scene——只命中该作用域内节点。
    /// 用于 CSS 作用域隔离测试。
    fn push_scoped(s: &mut Scene, scope_root: NodeId, r: DynamicRule) {
        s.dynamic_rules.entries.push(ScopedRule {
            rule: r,
            scope_root,
        });
    }

    /// 构造 root + button(.btn) scene，button 在 (0,0,100,100)。
    fn btn_scene() -> Scene {
        let mut root = Node::default();
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut btn = Node::default();
        btn.kind = NodeKind::Button;
        btn.classes = vec!["btn".to_string()];
        btn.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        Scene::from_nodes(vec![root, btn], vec![(0, 1)])
    }

    /// btn 的 NodeId（root 的唯一子）。
    fn btn_id(s: &Scene) -> NodeId {
        s.get(s.roots[0]).unwrap().children[0]
    }

    /// Construct a ParsedSelector manually (no parse dependency).
    /// Supports: .class, #id, :hover/:active/:disabled/:focus,
    /// [attr], [attr="val"], tag, and descendant combinator (space).
    fn hand_selector(sel: &str) -> ParsedSelector {
        let raw = sel.to_string();
        let mut compounds = Vec::new();
        for part in sel.split_whitespace() {
            let mut c = Compound {
                tag: None,
                classes: Vec::new(),
                id: None,
                combinator: Combinator::Descendant,
                pseudo_hover: false,
                pseudo_active: false,
                pseudo_disabled: false,
                pseudo_focus: false,
                pseudo_nth_child: None,
                attrs: Vec::new(),
                part: None,
            };
            let mut rest = part;
            while !rest.is_empty() {
                if rest.starts_with('.') {
                    let r = &rest[1..];
                    let end = r
                        .find(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
                        .unwrap_or(r.len());
                    c.classes.push(r[..end].to_string());
                    rest = &r[end..];
                } else if rest.starts_with('#') {
                    let r = &rest[1..];
                    let end = r
                        .find(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
                        .unwrap_or(r.len());
                    c.id = Some(r[..end].to_string());
                    rest = &r[end..];
                } else if let Some(r) = rest.strip_prefix(":hover") {
                    c.pseudo_hover = true;
                    rest = r;
                } else if let Some(r) = rest.strip_prefix(":active") {
                    c.pseudo_active = true;
                    rest = r;
                } else if let Some(r) = rest.strip_prefix(":disabled") {
                    c.pseudo_disabled = true;
                    rest = r;
                } else if let Some(r) = rest.strip_prefix(":focus") {
                    c.pseudo_focus = true;
                    rest = r;
                } else if rest.starts_with('[') {
                    let close = rest.find(']').unwrap();
                    let inner = &rest[1..close];
                    if let Some(eq) = inner.find('=') {
                        c.attrs.push(AttrSelector {
                            name: inner[..eq].to_string(),
                            op: AttrOp::Eq,
                            value: Some(inner[eq + 1..].trim_matches('"').to_string()),
                        });
                    } else {
                        c.attrs.push(AttrSelector {
                            name: inner.to_string(),
                            op: AttrOp::Exists,
                            value: None,
                        });
                    }
                    rest = &rest[close + 1..];
                } else {
                    let end = rest.find(['.', '#', ':', '[']).unwrap_or(rest.len());
                    if end > 0 {
                        c.tag = Some(rest[..end].to_string());
                    }
                    rest = &rest[end..];
                }
            }
            compounds.push(c);
        }
        ParsedSelector {
            raw,
            compound: compounds,
            specificity: Specificity(0, 0, 0),
        }
    }

    fn rule(sel: &str, prop: &str, val: &str) -> DynamicRule {
        DynamicRule {
            selector: hand_selector(sel),
            declarations: vec![Declaration {
                prop: prop.to_string(),
                value: val.to_string(),
            }],
        }
    }

    /// CustomElement tag 选择器：`game-item-card { ... }` 命中带 custom_tag 字面值的 host
    ///（pkg v35 组件展开保留）；`div` 不命中（tag 匹配走字面值而非 kind 逆映射）。
    #[test]
    fn custom_tag_selector_matches_host() {
        let mut s = btn_scene();
        let hid = btn_id(&s);
        {
            let n = s.get_mut(hid).unwrap();
            n.kind = NodeKind::CustomElement;
            n.custom_tag = Some("game-item-card".to_string());
        }
        push_global(
            &mut s,
            rule("game-item-card", "background-color", "#0000ff"),
        );
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(hid).unwrap().style.background_color,
            Some([0.0, 0.0, 1.0, 1.0]),
            "hyphen tag selector matches custom_tag literal"
        );
    }

    #[test]
    fn custom_tag_selector_div_does_not_match_host() {
        let mut s = btn_scene();
        let hid = btn_id(&s);
        {
            let n = s.get_mut(hid).unwrap();
            n.kind = NodeKind::CustomElement;
            n.custom_tag = Some("game-item-card".to_string());
        }
        push_global(&mut s, rule("div", "background-color", "#ff0000"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(hid).unwrap().style.background_color,
            None,
            "div selector must NOT match a tagged CustomElement"
        );
    }

    #[test]
    fn hover_pseudo_changes_background_color() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn:hover", "background-color", "#0000ff"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED); // 模拟命中 diff 后状态
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        // background_color 是视觉字段，不触发 layout dirty
        assert_eq!(
            s.get(bid).unwrap().style.background_color,
            Some([0.0, 0.0, 1.0, 1.0]),
            "hover → 蓝"
        );
    }

    /// #74 `<a>` hover 全链：a 节点（UA 烙印 base_style：链接色 #0000EE + INH_COLOR
    /// bit + text_decoration underline）→ `a:hover { color }` 规则命中（tag 选择器
    /// "a" 映射 Link）→ rematch 覆盖 UA 色（作者 > UA）→ rich run 重编译吃到 hover 色。
    /// 同时守卫：hover 色不得被 propagate_inherited 拿父值洗掉（INH_COLOR bit 在），
    /// 非 hover 态保持 UA 蓝。
    #[test]
    fn link_hover_rematch_recolors_rich_run() {
        use crate::text::rich_compile::compile_rich_runs;
        let mut root = Node::default();
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut div = Node::default();
        div.rich_text_block = true;
        div.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 150.0,
            h: 40.0,
        };
        // a 的 base_style 模拟打包期 UA 烙印：#0000EE + INH_COLOR bit + underline。
        let mut a = Node::default();
        a.kind = NodeKind::Link;
        a.base_style.color = [0.0, 0.0, 238.0 / 255.0, 1.0];
        a.base_style.text_decoration = crate::style::resolved::TextDecoration::Underline;
        let color_bit = inherited_bit("color").unwrap();
        a.base_style.inherited_set.0 |= color_bit;
        let mut tn = Node::default();
        tn.kind = NodeKind::TextNode;
        let mut s = Scene::from_nodes(vec![root, div, a, tn], vec![(0, 1), (1, 2), (2, 3)]);
        let div_id = s.get(s.roots[0]).unwrap().children[0];
        let a_id = s.get(div_id).unwrap().children[0];
        s.text_contents.insert(
            *s.get(a_id).unwrap().children.first().unwrap(),
            "商店".into(),
        );
        s.link_hrefs.insert(a_id, "open-shop".into());

        push_global(&mut s, rule("a:hover", "color", "#ff0000"));
        let sizes = std::collections::HashMap::new();

        // 非 hover 态：rematch 从 base_style 起，无规则命中 → UA 蓝保真。
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(a_id).unwrap().style.color,
            [0.0, 0.0, 238.0 / 255.0, 1.0]
        );
        let runs = compile_rich_runs(&s, div_id, &sizes);
        let link_run = runs.iter().find(|r| r.source == a_id).expect("链接 run");
        assert_eq!(
            link_run.color,
            [0.0, 0.0, 238.0 / 255.0, 1.0],
            "UA 蓝进 run"
        );
        assert_eq!(link_run.link_id, Some(a_id.0 as u32));
        assert!(link_run.deco.lines.underline(), "UA underline 进 run");

        // hover 态：`a:hover` 覆盖 UA 色 → run 重编译吃到 hover 红。
        s.get_mut(a_id)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(a_id).unwrap().style.color,
            [1.0, 0.0, 0.0, 1.0],
            "hover 规则色覆盖 UA 蓝（作者 > UA）"
        );
        let runs = compile_rich_runs(&s, div_id, &sizes);
        let link_run = runs.iter().find(|r| r.source == a_id).expect("链接 run");
        assert_eq!(link_run.color, [1.0, 0.0, 0.0, 1.0], "hover 红进重编译 run");
        assert_eq!(link_run.link_id, Some(a_id.0 as u32), "link_id 不变");
    }

    #[test]
    fn child_text_inherits_parent_runtime_color() {
        // root → parent(.par) → Text child。模拟打包期：parent base color=灰（.par 声明），
        // child base color=灰（打包期继承 parent base）。.par:hover 命中改 parent color 深。
        // CSS color 继承：child Text 无自己 color 声明 → runtime 该继承 parent runtime 深。
        let mut root = Node::default();
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut parent = Node::default();
        parent.classes = vec!["par".to_string()];
        parent.base_style.color = [0.6, 0.63, 0.71, 1.0]; // 灰（.par 声明）
        parent.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let mut child = Node::default();
        child.kind = NodeKind::TextNode;
        child.base_style.color = [0.6, 0.63, 0.71, 1.0]; // 灰（打包期继承 parent base）
        child.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 20.0,
        };
        let mut s = Scene::from_nodes(vec![root, parent, child], vec![(0, 1), (1, 2)]);
        push_global(&mut s, rule(".par:hover", "color", "#1a1d2e"));
        let pid = s.get(s.roots[0]).unwrap().children[0];
        s.get_mut(pid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        let cid = s.get(pid).unwrap().children[0];
        let c = s.get(cid).unwrap().style.color;
        // #1a1d2e = (26,29,46)/255
        assert!(
            (c[0] - 26.0 / 255.0).abs() < 1e-3
                && (c[1] - 29.0 / 255.0).abs() < 1e-3
                && (c[2] - 46.0 / 255.0).abs() < 1e-3,
            "child Text 该继承 parent hover color (#1a1d2e)，实际 {:?}",
            c
        );
    }

    #[test]
    fn child_inherits_parent_font_size() {
        // root(.par font-size:24) > Text child。child 无 font-size 规则 → 该继承 24。
        // 证明通用继承（非 color-only）。
        let mut root = Node::default();
        root.classes = vec!["par".to_string()];
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut child = Node::default();
        child.kind = NodeKind::TextNode;
        child.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 20.0,
        };
        let mut s = Scene::from_nodes(vec![root, child], vec![(0, 1)]);
        push_global(&mut s, rule(".par", "font-size", "24px"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        let cid = s.get(s.roots[0]).unwrap().children[0];
        assert_eq!(
            s.get(cid).unwrap().style.font_size,
            24.0,
            "child Text 该继承 parent .par 的 font-size:24"
        );
    }

    #[test]
    fn child_explicit_font_size_not_overridden_by_inheritance() {
        // child 自己声明 font-size:12 → 不被父的 24 覆盖（set-ness 阻断继承）。
        let mut root = Node::default();
        root.classes = vec!["par".to_string()];
        let mut child = Node::default();
        child.classes = vec!["c".to_string()];
        child.kind = NodeKind::TextNode;
        let mut s = Scene::from_nodes(vec![root, child], vec![(0, 1)]);
        push_global(&mut s, rule(".par", "font-size", "24px"));
        push_global(&mut s, rule(".c", "font-size", "12px"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        let cid = s.get(s.roots[0]).unwrap().children[0];
        assert_eq!(
            s.get(cid).unwrap().style.font_size,
            12.0,
            "child 显式声明 12 不被继承覆盖"
        );
    }

    #[test]
    fn inheritance_cascades_two_levels() {
        // root(.a font-size:20) > mid > leaf(Text)。mid/leaf 都不声明 → leaf 继承 20（跨两级）。
        let mut root = Node::default();
        root.classes = vec!["a".to_string()];
        let mid = Node::default();
        let mut leaf = Node::default();
        leaf.kind = NodeKind::TextNode;
        let mut s = Scene::from_nodes(vec![root, mid, leaf], vec![(0, 1), (1, 2)]);
        push_global(&mut s, rule(".a", "font-size", "20px"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        let mid_id = s.get(s.roots[0]).unwrap().children[0];
        let leaf_id = s.get(mid_id).unwrap().children[0];
        assert_eq!(
            s.get(leaf_id).unwrap().style.font_size,
            20.0,
            "leaf 跨级继承 20"
        );
    }

    #[test]
    fn active_pseudo_on_down() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn:active", "background-color", "#ff0000"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::ACTIVE);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.background_color,
            Some([1.0, 0.0, 0.0, 1.0]),
            "active → 红"
        );
    }

    #[test]
    fn disabled_pseudo_via_disabled_flag() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn:disabled", "opacity", "0.5"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::DISABLED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.opacity,
            0.5,
            "disabled → opacity 0.5"
        );
    }

    #[test]
    fn rematch_layout_dirty_when_size_changes() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn:hover", "width", "200px"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        // 验 style.taffy_style.size.width 被改
        use taffy::style::Dimension;
        assert_eq!(
            s.get(bid).unwrap().style.taffy_style.size.width,
            Dimension::length(200.0)
        );
    }

    #[test]
    fn rematch_no_dirty_when_only_visual_changes() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn:hover", "color", "#ff0000"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn descendant_pseudo_rule_matched() {
        // .parent:hover .child —— hover parent → child style 变（跨节点伪类联动）
        let mut root = Node::default();
        root.classes = vec!["parent".to_string()];
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut child = Node::default();
        child.classes = vec!["child".to_string()];
        child.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 50.0,
        };
        let mut s = Scene::from_nodes(vec![root, child], vec![(0, 1)]);
        let root_id = s.roots[0];
        let child_id = s.get(root_id).unwrap().children[0];
        push_global(&mut s, rule(".parent:hover .child", "color", "#0000ff"));
        s.get_mut(root_id)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED); // parent hovered
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(child_id).unwrap().style.color,
            [0.0, 0.0, 1.0, 1.0],
            "parent:hover → child 变蓝"
        );
    }

    #[test]
    fn child_combinator_matches_direct_parent_only() {
        // #114：`>` 限直接父。三链 gp(.gp) → p(.p) → c(.c)：
        // `.p > .c` 命中；`.gp > .c` 不命中（c 的父是 p）；`.gp .c` 命中（Descendant 回归）。
        // hand_selector 只造 Descendant，Child 在此手工标注——解析端（fence）另有全链测试。
        let mk = |sel: &str, child_comb: bool| {
            let mut s = hand_selector(sel);
            if child_comb {
                s.raw = sel.replace(' ', " > ");
                s.compound[1].combinator = Combinator::Child;
            }
            s
        };
        let mut gp = Node::default();
        gp.classes = vec!["gp".to_string()];
        let mut p = Node::default();
        p.classes = vec!["p".to_string()];
        let mut c = Node::default();
        c.classes = vec!["c".to_string()];
        let mut s = Scene::from_nodes(vec![gp, p, c], vec![(0, 1), (1, 2)]);
        let gp_id = s.roots[0];
        let p_id = s.get(gp_id).unwrap().children[0];
        let c_id = s.get(p_id).unwrap().children[0];

        push_global(
            &mut s,
            DynamicRule {
                selector: mk(".p .c", true),
                declarations: vec![Declaration {
                    prop: "color".to_string(),
                    value: "#ff0000".to_string(),
                }],
            },
        );
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(c_id).unwrap().style.color,
            [1.0, 0.0, 0.0, 1.0],
            "直父命中 → 红"
        );

        // .gp > .c：祖父非直父，Child 不命中
        let mut s = Scene::from_nodes(
            vec![Node::default(), Node::default(), Node::default()],
            vec![(0, 1), (1, 2)],
        );
        let gp_id = s.roots[0];
        let p_id = s.get(gp_id).unwrap().children[0];
        let c_id = s.get(p_id).unwrap().children[0];
        s.get_mut(gp_id).unwrap().classes = vec!["gp".to_string()];
        s.get_mut(p_id).unwrap().classes = vec!["p".to_string()];
        s.get_mut(c_id).unwrap().classes = vec!["c".to_string()];
        push_global(
            &mut s,
            DynamicRule {
                selector: mk(".gp .c", true),
                declarations: vec![Declaration {
                    prop: "color".to_string(),
                    value: "#ff0000".to_string(),
                }],
            },
        );
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_ne!(
            s.get(c_id).unwrap().style.color,
            [1.0, 0.0, 0.0, 1.0],
            "祖父隔代 → Child 不命中"
        );

        // .gp .c：同结构 Descendant 命中——锁 Child/Descendant 语义区分
        push_global(&mut s, rule(".gp .c", "color", "#00ff00"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(c_id).unwrap().style.color,
            [0.0, 1.0, 0.0, 1.0],
            "同结构 Descendant 命中 → 绿"
        );
    }

    #[test]
    fn no_pseudo_rule_not_in_dynamic_rules() {
        // 纯静态规则不进 dynamic_rules（打包器分流）——rematch 不区分有无伪类，
        // 只看状态门。若纯静态规则混进 dynamic，hovered=true 时仍匹配（无伪类规则恒匹配）。
        // 打包器保证无伪类规则不进 dynamic_rules。此测断言 color 变红。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn", "color", "#ff0000"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn rematch_resets_to_base_when_no_rule_matches() {
        // hover 后变蓝 → hover 取消 → rematch 应回 base_style
        let mut s = btn_scene();
        let bid = btn_id(&s);
        s.get_mut(bid).unwrap().base_style.background_color = None; // base 无 bg
        push_global(&mut s, rule(".btn:hover", "background-color", "#0000ff"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.background_color,
            Some([0.0, 0.0, 1.0, 1.0])
        );
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .remove(NodeFlags::HOVERED); // 取消 hover
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.background_color,
            None,
            "取消 hover → 回 base"
        );
    }

    #[test]
    fn focus_pseudo_matches_focused_node() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn:focus", "background-color", "#0000ff"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::FOCUSED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.background_color,
            Some([0.0, 0.0, 1.0, 1.0]),
            "focused → :focus 匹配 → 蓝"
        );
    }

    #[test]
    fn focus_pseudo_no_match_unfocused() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        s.get_mut(bid).unwrap().base_style.background_color = None;
        push_global(&mut s, rule(".btn:focus", "background-color", "#0000ff"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .remove(NodeFlags::FOCUSED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.background_color,
            None,
            "unfocused → :focus 不匹配 → 回 base"
        );
    }

    #[test]
    fn focus_pseudo_in_descendant_chain() {
        // .parent:focus .child —— parent 聚焦 → child style 变
        let mut root = Node::default();
        root.classes = vec!["parent".to_string()];
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut child = Node::default();
        child.classes = vec!["child".to_string()];
        child.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 50.0,
        };
        let mut s = Scene::from_nodes(vec![root, child], vec![(0, 1)]);
        let root_id = s.roots[0];
        let child_id = s.get(root_id).unwrap().children[0];
        push_global(&mut s, rule(".parent:focus .child", "color", "#0000ff"));
        s.get_mut(root_id)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::FOCUSED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(child_id).unwrap().style.color,
            [0.0, 0.0, 1.0, 1.0],
            "parent:focus → child 变蓝"
        );
    }

    #[test]
    fn background_image_change_is_visual_not_layout_dirty() {
        // background-image 是视觉字段（非 taffy_style/order）→ rematch 不 layout dirty
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(
            &mut s,
            rule(".btn:hover", "background-image", "url(icons/home.png)"),
        );
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.background_image.as_deref(),
            Some("icons/home.png"),
            "hover → background-image 生效"
        );
    }

    #[test]
    fn background_size_change_is_visual_not_layout_dirty() {
        // background-size 是视觉字段 → rematch 不 layout dirty
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn:hover", "background-size", "cover"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.background_size,
            crate::style::resolved::BackgroundSize::Cover,
            "hover → background-size:cover 生效"
        );
    }

    #[test]
    fn border_radius_change_is_visual_not_layout_dirty() {
        // border-radius 是视觉字段（非 taffy_style/order）→ rematch 不 layout dirty
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn:hover", "border-radius", "8px"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        // hover → border-radius:8px 生效（四角 h=v=Length(8)）
        let bc = &s.get(bid).unwrap().style.border_radius.corners;
        for c in bc {
            assert_eq!(
                c.h,
                taffy::style::LengthPercentage::length(8.0),
                "hover → border-radius 水平 8px"
            );
            assert_eq!(
                c.v,
                taffy::style::LengthPercentage::length(8.0),
                "hover → border-radius 垂直 8px"
            );
        }
    }

    #[test]
    fn attr_selector_bincode_roundtrip() {
        let s = hand_selector(r#"[data-page="1"]"#);
        assert_eq!(s.compound.len(), 1);
        assert_eq!(s.compound[0].attrs.len(), 1, "[data-page=\"1\"] → one attr");
        let a = &s.compound[0].attrs[0];
        assert_eq!(a.name, "data-page");
        assert!(matches!(a.op, AttrOp::Eq));
        assert_eq!(a.value.as_deref(), Some("1"));
        // bincode roundtrip (pkg.bin dynamic blob is bincode)
        let bytes = bincode::serialize(&s).unwrap();
        let back: ParsedSelector = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.compound[0].attrs.len(), 1);
        assert_eq!(back.compound[0].attrs[0].name, "data-page");
    }

    /// 极简 Node 构造（仅设 kind）。attr selector 测试不需要 layout_rect/scene。
    fn test_node(kind: NodeKind) -> Node {
        let mut n = Node::default();
        n.kind = kind;
        n
    }

    /// 单节点 scene（root = 该节点）。compound_matches_node 现需 scene+id（attr 匹配读
    /// ControlState/RoleTable），故 attr/tag 测试经此包装。
    fn single_node_scene(node: Node) -> (Scene, NodeId) {
        let s = Scene::from_nodes(vec![node], vec![]);
        let id = s.roots[0];
        (s, id)
    }

    /// 单节点 scene + 注入 ControlState（aria 合成测试用）。
    fn control_scene(kind: NodeKind, state: ControlState) -> (Scene, NodeId) {
        let mut s = Scene::from_nodes(vec![test_node(kind)], vec![]);
        let id = s.roots[0];
        s.controls.ensure(id, state);
        (s, id)
    }

    /// 单节点 scene + 注入 RoleInfo（role/data-slot 匹配测试用）。slots 复刻 instantiate
    /// 路径：key=slot 名，值空串占位（`data-slot="thumb"` → slots["thumb"]=""）。
    fn role_scene(role: Option<&str>, slots: &[&str]) -> (Scene, NodeId) {
        let mut s = Scene::from_nodes(vec![test_node(NodeKind::Container)], vec![]);
        let id = s.roots[0];
        let info = RoleInfo {
            role: role.map(String::from),
            slots: slots
                .iter()
                .map(|&k| (k.to_string(), String::new()))
                .collect(),
            attrs: vec![],
        };
        s.roles.insert(id, info);
        (s, id)
    }

    #[test]
    fn attr_selector_type_matches_nodekind_precisely() {
        // [type="text"] 只匹配 TextField，不匹配 NumberField
        let sel = hand_selector(r#"[type="text"]"#);
        let (s_text, text) = single_node_scene(test_node(NodeKind::TextField));
        let (s_num, num) = single_node_scene(test_node(NodeKind::NumberField));
        assert!(compound_matches_node(&sel.compound[0], text, &s_text));
        assert!(!compound_matches_node(&sel.compound[0], num, &s_num));
        // [type="number"] 只匹配 NumberField
        let sel_num = hand_selector(r#"[type="number"]"#);
        assert!(compound_matches_node(&sel_num.compound[0], num, &s_num));
        assert!(!compound_matches_node(&sel_num.compound[0], text, &s_text));
    }

    #[test]
    fn tag_selector_matches_nodekind_roundtrip() {
        // 标签选择器 ↔ NodeKind 互逆回归：fence resolve_semantic 放行的、且在运行时成为 Node
        // 的每个 tag 都须在 rematch 命中。这里覆盖曾误退回 "div" 的 ListView/Slot，
        // 以及控件回溯 tag（progress）与基础 tag 作为健全性检查。
        //
        // 注意 `<template>` 不在此列：ListView 蓝图的 <template> 子树自 pkg v27 起进运行时
        // （NodeKind::Template，强制 display:none，tag 选择器可命中），不属常规可见 tag，
        // 此处不重复覆盖。
        // fence tag → NodeKind 真相源：crates/fence/src/schema/tag.rs::resolve_semantic
        let cases: &[(&str, NodeKind)] = &[
            ("ul", NodeKind::ListView),
            ("slot", NodeKind::Slot),
            ("progress", NodeKind::ProgressBar),
            ("div", NodeKind::Container),
        ];
        for (tag, kind) in cases {
            let sel = hand_selector(tag);
            let (s, id) = single_node_scene(test_node(*kind));
            assert!(
                compound_matches_node(&sel.compound[0], id, &s),
                "tag 选择器 `{tag}` 应命中 NodeKind::{kind:?}（rematch 路径）"
            );
        }
        // 负向：`ul` 不该命中 Container（防误退回 div 后误匹配）
        let ul_sel = hand_selector("ul");
        let (s_div, div) = single_node_scene(test_node(NodeKind::Container));
        assert!(
            !compound_matches_node(&ul_sel.compound[0], div, &s_div),
            "`ul` 不应命中 Container"
        );
    }

    #[test]
    fn attr_selector_non_type_attr_does_not_match() {
        // 非已知 attr（type/aria-*/role/data-slot）：不匹配（Node 不携带任意 HTML 属性字面值）
        let sel = hand_selector("[disabled]");
        let (s, id) = single_node_scene(test_node(NodeKind::TextField));
        assert!(!compound_matches_node(&sel.compound[0], id, &s));
    }

    #[test]
    fn attr_selector_type_exists_form_does_not_match() {
        // [type] 存在形式（无 = val）：不支持（type 是结构性属性，存在性无匹配意义）
        let sel = hand_selector("[type]");
        let (s, id) = single_node_scene(test_node(NodeKind::TextField));
        assert!(!compound_matches_node(&sel.compound[0], id, &s));
    }

    #[test]
    fn attr_matches_aria_checked_from_toggle() {
        // Toggle{checked:true} → [aria-checked="true"] 命中；翻成 false → [aria-checked="false"] 命中
        let (mut s, id) = control_scene(NodeKind::Toggle, ControlState::Toggle { checked: true });
        let sel_true = hand_selector(r#"[aria-checked="true"]"#);
        let sel_false = hand_selector(r#"[aria-checked="false"]"#);
        assert!(
            compound_matches_node(&sel_true.compound[0], id, &s),
            "checked:true → [aria-checked=\"true\"] 命中"
        );
        assert!(
            !compound_matches_node(&sel_false.compound[0], id, &s),
            "checked:true → [aria-checked=\"false\"] 不命中"
        );
        // flip → 实时合成值变
        s.controls
            .ensure(id, ControlState::Toggle { checked: false });
        assert!(
            !compound_matches_node(&sel_true.compound[0], id, &s),
            "checked:false → [aria-checked=\"true\"] 不命中"
        );
        assert!(
            compound_matches_node(&sel_false.compound[0], id, &s),
            "checked:false → [aria-checked=\"false\"] 命中"
        );
    }

    #[test]
    fn attr_matches_aria_checked_from_radio() {
        // Radio{checked:true} → [aria-checked="true"] 命中（与 Toggle 共用 aria-checked）
        let (s, id) = control_scene(
            NodeKind::RadioButton,
            ControlState::Radio {
                checked: true,
                name: "grp".into(),
            },
        );
        let sel = hand_selector(r#"[aria-checked="true"]"#);
        assert!(
            compound_matches_node(&sel.compound[0], id, &s),
            "Radio checked → [aria-checked=\"true\"] 命中"
        );
    }

    #[test]
    fn attr_matches_aria_checked_no_state_no_match() {
        // 无 ControlState 的普通 div → aria-checked 不合成 → Eq / Exists 均不命中
        let (s, id) = single_node_scene(test_node(NodeKind::Container));
        let sel_eq = hand_selector(r#"[aria-checked="true"]"#);
        assert!(!compound_matches_node(&sel_eq.compound[0], id, &s));
        let sel_exists = hand_selector("[aria-checked]");
        assert!(
            !compound_matches_node(&sel_exists.compound[0], id, &s),
            "[aria-checked] 存在形式：普通 div 无 aria-checked 语义"
        );
    }

    #[test]
    fn attr_matches_aria_expanded_from_dropdown() {
        // Dropdown{open:true} → [aria-expanded="true"] 命中
        let (s, id) = control_scene(
            NodeKind::Dropdown,
            ControlState::Dropdown {
                selected_index: 0,
                open: true,
                value_lock: false,
                open_selected_index: None,
                option_values: Vec::new(),
            },
        );
        let sel = hand_selector(r#"[aria-expanded="true"]"#);
        assert!(
            compound_matches_node(&sel.compound[0], id, &s),
            "Dropdown open → [aria-expanded=\"true\"] 命中"
        );
        let sel_false = hand_selector(r#"[aria-expanded="false"]"#);
        assert!(!compound_matches_node(&sel_false.compound[0], id, &s));
    }

    #[test]
    fn synth_treeitem_expanded_branch_only() {
        // leaf treeitem 持态（指针激活目标解析需要——见 bridge 烘焙/回归
        // tree_leaf_hit_refines_to_leaf_not_parent_branch），但 aria-expanded 合成仅对
        // branch 有语义：ARIA 不给 leaf 上 aria-expanded，web 预览模拟器同口径。
        use crate::scene::dynamic::{append_child, create_node_from_template};
        use crate::style::resolved::ResolvedStyle;
        let mut s = Scene::default();
        let branch =
            create_node_from_template(&mut s, NodeKind::TreeItem, ResolvedStyle::default(), None);
        let leaf =
            create_node_from_template(&mut s, NodeKind::TreeItem, ResolvedStyle::default(), None);
        append_child(&mut s, branch, leaf).unwrap();
        s.controls
            .ensure(branch, ControlState::TreeItem { expanded: true });
        s.controls
            .ensure(leaf, ControlState::TreeItem { expanded: false });
        assert_eq!(
            synth_aria_value(&s, branch, "expanded"),
            Some("true".to_string())
        );
        assert_eq!(
            synth_aria_value(&s, leaf, "expanded"),
            None,
            "leaf 持态但不合成 aria-expanded"
        );
    }

    #[test]
    fn attr_matches_aria_valuenow_from_progress() {
        // Progress{value:50.0} → [aria-valuenow="50"] 命中（f32 Display：50.0→"50"）
        let (s, id) = control_scene(
            NodeKind::ProgressBar,
            ControlState::Progress {
                value: 50.0,
                min: 0.0,
                max: 100.0,
                indeterminate: false,
            },
        );
        let sel = hand_selector(r#"[aria-valuenow="50"]"#);
        assert!(
            compound_matches_node(&sel.compound[0], id, &s),
            "Progress 50.0 → [aria-valuenow=\"50\"] 命中"
        );
    }

    #[test]
    fn attr_matches_aria_valuemin_from_progress() {
        // Progress{min:50.0} → [aria-valuemin="50"] 命中（运行时改 min 后属性选择器同拍镜像）。
        let (s, id) = control_scene(
            NodeKind::ProgressBar,
            ControlState::Progress {
                value: 75.0,
                min: 50.0,
                max: 100.0,
                indeterminate: false,
            },
        );
        let sel = hand_selector(r#"[aria-valuemin="50"]"#);
        assert!(
            compound_matches_node(&sel.compound[0], id, &s),
            "Progress min=50.0 → [aria-valuemin=\"50\"] 命中"
        );
    }

    #[test]
    fn attr_matches_aria_indeterminate_from_progress() {
        // indeterminate=true → [aria-indeterminate="true"] 命中。入口 = 打包期
        // aria-valuenow 缺席（ARIA 语义）；运行时 C# set_is_indeterminate 后靠本合成
        // 镜像让作者 CSS 感知状态翻转。
        let (s, id) = control_scene(
            NodeKind::ProgressBar,
            ControlState::Progress {
                value: 0.0,
                min: 0.0,
                max: 100.0,
                indeterminate: true,
            },
        );
        let sel = hand_selector(r#"[aria-indeterminate="true"]"#);
        assert!(
            compound_matches_node(&sel.compound[0], id, &s),
            "indeterminate → [aria-indeterminate=\"true\"] 命中"
        );
        let sel_false = hand_selector(r#"[aria-indeterminate="false"]"#);
        assert!(!compound_matches_node(&sel_false.compound[0], id, &s));
    }

    #[test]
    fn attr_matches_aria_valuenow_from_slider() {
        // Slider{value:33.5} → [aria-valuenow="33.5"] 命中
        let (s, id) = control_scene(
            NodeKind::Slider,
            ControlState::Slider {
                value: 33.5,
                min: 0.0,
                max: 100.0,
                step: 0.5,
                dragging: false,
            },
        );
        let sel = hand_selector(r#"[aria-valuenow="33.5"]"#);
        assert!(
            compound_matches_node(&sel.compound[0], id, &s),
            "Slider 33.5 → [aria-valuenow=\"33.5\"] 命中"
        );
    }

    #[test]
    fn attr_matches_aria_valuenow_from_numberfield() {
        // NumberField{edit.value="42"} → [aria-valuenow="42"] 命中（数值文本直传）
        let (s, id) = control_scene(
            NodeKind::NumberField,
            ControlState::NumberField {
                edit: EditState::from_init("42".into(), String::new(), 0, false),
                min: 0.0,
                max: 100.0,
                step: 1.0,
            },
        );
        let sel = hand_selector(r#"[aria-valuenow="42"]"#);
        assert!(
            compound_matches_node(&sel.compound[0], id, &s),
            "NumberField value=42 → [aria-valuenow=\"42\"] 命中"
        );
    }

    #[test]
    fn attr_matches_aria_multiline_from_textarea() {
        // aria-multiline 静态：TextArea → "true"；TextField → 不合成（单行无 multiline 语义）
        let (s_ta, ta) = control_scene(
            NodeKind::TextArea,
            ControlState::TextArea(EditState::from_init(String::new(), String::new(), 0, false)),
        );
        let sel = hand_selector(r#"[aria-multiline="true"]"#);
        assert!(
            compound_matches_node(&sel.compound[0], ta, &s_ta),
            "TextArea → [aria-multiline=\"true\"] 命中"
        );
        let (s_tf, tf) = control_scene(
            NodeKind::TextField,
            ControlState::TextField(EditState::from_init(String::new(), String::new(), 0, false)),
        );
        assert!(
            !compound_matches_node(&sel.compound[0], tf, &s_tf),
            "TextField → [aria-multiline=\"true\"] 不命中"
        );
    }

    #[test]
    fn attr_matches_role_eq_and_exists() {
        let (s, id) = role_scene(Some("switch"), &[]);
        let sel_eq = hand_selector(r#"[role="switch"]"#);
        assert!(
            compound_matches_node(&sel_eq.compound[0], id, &s),
            "[role=\"switch\"] 命中 role=switch"
        );
        let sel_other = hand_selector(r#"[role="slider"]"#);
        assert!(
            !compound_matches_node(&sel_other.compound[0], id, &s),
            "[role=\"slider\"] 不命中 switch"
        );
        let sel_exists = hand_selector("[role]");
        assert!(
            compound_matches_node(&sel_exists.compound[0], id, &s),
            "[role] 存在形式命中"
        );
        let (s_div, div) = role_scene(None, &[]);
        assert!(
            !compound_matches_node(&sel_exists.compound[0], div, &s_div),
            "无 role 的 div → [role] 不命中"
        );
    }

    #[test]
    fn attr_matches_data_slot_eq_and_exists() {
        let (s, id) = role_scene(None, &["thumb"]);
        let sel_eq = hand_selector(r#"[data-slot="thumb"]"#);
        assert!(
            compound_matches_node(&sel_eq.compound[0], id, &s),
            "[data-slot=\"thumb\"] 命中"
        );
        let sel_other = hand_selector(r#"[data-slot="fill"]"#);
        assert!(
            !compound_matches_node(&sel_other.compound[0], id, &s),
            "[data-slot=\"fill\"] 不命中 thumb 节点"
        );
        let sel_exists = hand_selector("[data-slot]");
        assert!(
            compound_matches_node(&sel_exists.compound[0], id, &s),
            "[data-slot] 存在形式命中"
        );
    }

    #[test]
    fn aria_checked_rematch_drives_style() {
        // 端到端：Toggle + [aria-checked="true"]{color:red}。checked:true → 染红；翻 false → 回落。
        let mut root = Node::default();
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let mut s = Scene::from_nodes(vec![root], vec![]);
        let id = s.roots[0];
        s.controls
            .ensure(id, ControlState::Toggle { checked: true });
        push_global(&mut s, rule(r#"[aria-checked="true"]"#, "color", "#ff0000"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(id).unwrap().style.color,
            [1.0, 0.0, 0.0, 1.0],
            "checked:true → [aria-checked=\"true\"] 染红"
        );
        s.controls
            .ensure(id, ControlState::Toggle { checked: false });
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_ne!(
            s.get(id).unwrap().style.color,
            [1.0, 0.0, 0.0, 1.0],
            "checked:false → 不再染红"
        );
    }

    #[test]
    fn rematch_emits_transition_request_on_change() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        s.get_mut(bid).unwrap().base_style.transition = vec![TransitionSpec {
            prop: Some(TweenProp::BgColor),
            duration: 0.3,
            ease: Ease::Linear,
            delay: 0.0,
        }];
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(crate::scene::node::NodeFlags::CASCALED); // 已 warmup
        push_global(&mut s, rule(".btn:hover", "background-color", "#0000ff"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        s.pending_transitions.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.pending_transitions.len(),
            1,
            "bg changed + transition declared → 1 request"
        );
        let r = &s.pending_transitions[0];
        assert!(matches!(r.prop, TweenProp::BgColor));
    }

    #[test]
    fn rematch_emits_transform_transition_request() {
        // .btn:hover 换 transform + transition:transform 声明 → Transform 复合通道请求，
        // end = 新矩阵的 TRS 分解（translate(10,20)·scale(2,1) → [10,20,2,1,0]）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        s.get_mut(bid).unwrap().base_style.transition = vec![TransitionSpec {
            prop: Some(TweenProp::Transform),
            duration: 0.2,
            ease: Ease::Linear,
            delay: 0.0,
        }];
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::CASCALED);
        push_global(
            &mut s,
            rule(
                ".btn:hover",
                "transform",
                "translate(10px, 20px) scale(2, 1)",
            ),
        );
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        s.pending_transitions.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.pending_transitions.len(), 1, "transform 变化 → 1 请求");
        let r = &s.pending_transitions[0];
        assert!(matches!(r.prop, TweenProp::Transform));
        let [tx, ty, sx, sy, rot] = r.end;
        assert!((tx - 10.0).abs() < 1e-5, "tx {tx}");
        assert!((ty - 20.0).abs() < 1e-5, "ty {ty}");
        assert!((sx - 2.0).abs() < 1e-5, "sx {sx}");
        assert!((sy - 1.0).abs() < 1e-5, "sy {sy}");
        assert!(rot.abs() < 1e-5, "rot {rot}");
    }

    #[test]
    fn transform_transition_start_uses_midflight_override() {
        // mid-flight：anim.transform 已有半程矩阵 → request.start 取其分解（连续无 snap），
        // 而非旧级联值（identity）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        s.get_mut(bid).unwrap().base_style.transition = vec![TransitionSpec {
            prop: Some(TweenProp::Transform),
            duration: 0.2,
            ease: Ease::Linear,
            delay: 0.0,
        }];
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::CASCALED);
        // 旧级联：translate(0,0)（identity）；mid-flight override：translate(4, 0)
        push_global(
            &mut s,
            rule(".btn:hover", "transform", "translate(10px, 0px)"),
        );
        s.anim.ensure(bid).transform = Some(crate::transform::from_translate(4.0, 0.0));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        s.pending_transitions.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.pending_transitions.len(), 1);
        let r = &s.pending_transitions[0];
        assert!(
            (r.start[0] - 4.0).abs() < 1e-5,
            "start tx=4（override），got {:?}",
            r.start
        );
        assert!((r.end[0] - 10.0).abs() < 1e-5, "end tx=10");
    }

    #[test]
    fn transform_transition_all_spec_covers_channel() {
        // transition:all（prop=None）也覆盖 transform 通道（CSS all 语义）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        s.get_mut(bid).unwrap().base_style.transition = vec![TransitionSpec {
            prop: None,
            duration: 0.2,
            ease: Ease::Linear,
            delay: 0.0,
        }];
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::CASCALED);
        push_global(&mut s, rule(".btn:hover", "transform", "rotate(90deg)"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        s.pending_transitions.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.pending_transitions.len(), 1);
        let r = &s.pending_transitions[0];
        assert!(matches!(r.prop, TweenProp::Transform));
        assert!(
            (r.end[4] - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "rot {r:?}"
        );
    }

    #[test]
    fn first_cascade_no_transition() {
        // cascaded_once=false → 首次 cascade 即时生效，不产 transition 请求
        let mut s = btn_scene();
        let bid = btn_id(&s);
        s.get_mut(bid).unwrap().base_style.transition = vec![TransitionSpec {
            prop: Some(TweenProp::BgColor),
            duration: 0.3,
            ease: Ease::Linear,
            delay: 0.0,
        }];
        // cascaded_once 保持 false
        push_global(&mut s, rule(".btn:hover", "background-color", "#0000ff"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.pending_transitions.len(),
            0,
            "first cascade instant (no transition)"
        );
        assert!(
            s.get(bid)
                .unwrap()
                .interaction
                .flags
                .contains(crate::scene::node::NodeFlags::CASCALED),
            "首次 cascade 后 cascaded_once 置 true"
        );
    }

    /// 建 root → child（child 为 TextNode，无自身 color 声明）。
    fn build_parent_child() -> (Scene, NodeId, NodeId) {
        let mut root = Node::default();
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut child = Node::default();
        child.kind = NodeKind::TextNode;
        child.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 20.0,
        };
        let s = Scene::from_nodes(vec![root, child], vec![(0, 1)]);
        let root_id = s.roots[0];
        let child_id = s.get(root_id).unwrap().children[0];
        (s, root_id, child_id)
    }

    /// 建 root-only scene（无子），用于 probe。
    fn build_simple_tree() -> (Scene, NodeId) {
        let mut root = Node::default();
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let s = Scene::from_nodes(vec![root], vec![]);
        let root_id = s.roots[0];
        (s, root_id)
    }

    #[test]
    fn inline_override_color_inherits_to_child() {
        // 父 inline 设 color:red；child 无自身 color 声明 → 该继承父 inline 值。
        // 验证 set_map 接收 inline 的继承 bit（经 propagate_inherited 传父子）。
        let (mut scene, root, child) = build_parent_child();
        crate::scene::dynamic::set_inline_override(&mut scene, root, "color:#ff0000").unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        let child_color = scene.get(child).unwrap().style.color;
        assert_eq!(
            child_color,
            [1.0, 0.0, 0.0, 1.0],
            "child 无 color 声明 → 继承父 inline red"
        );
        // 父自己也该是 red（inline 应用到自身）
        assert_eq!(scene.get(root).unwrap().style.color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn inline_override_unset_falls_back() {
        // set 后 unset → bit 清 → rematch 不应用 → 回 base_style 值（非 red）。
        let (mut scene, root, _child) = build_parent_child();
        crate::scene::dynamic::set_inline_override(&mut scene, root, "color:#ff0000").unwrap();
        crate::scene::dynamic::unset_inline_override(&mut scene, root, "color").unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        assert_ne!(
            scene.get(root).unwrap().style.color,
            [1.0, 0.0, 0.0, 1.0],
            "unset 后 color 回落（不再 red）"
        );
        // inline_set 的 color bit 应被清
        let set = scene.get(root).unwrap().inline_set.0;
        assert_eq!(set & INH_COLOR, 0, "INH_COLOR bit 清零");
    }

    #[test]
    fn spec3_probe_no_regress_when_no_inline() {
        // 没设 inline 的节点（inline_set == 0）：rematch 不 panic。
        let (mut scene, root) = build_simple_tree();
        assert_eq!(scene.get(root).unwrap().inline_set.0, 0);
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        // base color = 默认黑（ResolvedStyle::default）
        assert_eq!(scene.get(root).unwrap().style.color, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn inline_override_beats_dynamic_rule() {
        // inline 优先级 > dynamic rule。同节点 hover 规则设 blue，inline 设 red → red 胜。
        let (mut scene, root, _child) = build_parent_child();
        scene.get_mut(root).unwrap().classes = vec!["r".to_string()];
        push_global(&mut scene, rule(".r", "color", "#0000ff"));
        crate::scene::dynamic::set_inline_override(&mut scene, root, "color:#ff0000").unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            scene.get(root).unwrap().style.color,
            [1.0, 0.0, 0.0, 1.0],
            "inline red 胜过 dynamic blue"
        );
    }

    #[test]
    fn inline_override_background_color_non_inherited() {
        // 非继承属性 background-color：inline 设 → 仅本节点生效，不传子。
        let (mut scene, root, child) = build_parent_child();
        crate::scene::dynamic::set_inline_override(&mut scene, root, "background-color:#00ff00")
            .unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            scene.get(root).unwrap().style.background_color,
            Some([0.0, 1.0, 0.0, 1.0]),
            "root inline bg green"
        );
        // child 无 inline，background_color 不继承 → 仍 None
        assert_eq!(
            scene.get(child).unwrap().style.background_color,
            None,
            "background_color 非继承 → 不传子"
        );
    }

    #[test]
    fn inline_override_width_non_inherited() {
        // 非继承 taffy 字段 width：inline 设 → 应用到 style.taffy_style.size.width。
        let (mut scene, root, _child) = build_parent_child();
        crate::scene::dynamic::set_inline_override(&mut scene, root, "width:123px").unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        use taffy::style::Dimension;
        assert_eq!(
            scene.get(root).unwrap().style.taffy_style.size.width,
            Dimension::length(123.0)
        );
    }

    #[test]
    fn inline_override_display_copies_both_taffy_and_mode() {
        // inline display:none 应同时设 taffy_style.display=None + display_mode=None。
        // 验证 INLINE_DISPLAY bit 的双字段覆盖。
        let (mut scene, root, _child) = build_parent_child();
        crate::scene::dynamic::set_inline_override(&mut scene, root, "display:none").unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            scene.get(root).unwrap().style.taffy_style.display,
            taffy::Display::None,
            "inline display:none → taffy Display::None"
        );
        assert_eq!(
            scene.get(root).unwrap().style.display_mode,
            crate::style::resolved::DisplayMode::None,
            "inline display:none → display_mode=None"
        );
    }

    #[test]
    fn package_inline_display_beats_class_rule() {
        // 打包期 inline style display:none（base_style + inline_declared 标记）不被 class 规则
        // .r{display:flex} 覆盖（CSS inline > class）。回归 showcase/shop dialog-overlay：class
        // display:flex + inline display:none 被错误覆盖成 flex，全屏 overlay 吞掉 back-home 点击。
        let (mut scene, root, _child) = build_parent_child();
        scene.get_mut(root).unwrap().classes = vec!["r".to_string()];
        {
            // 模拟打包期 inline display:none：base_style 双字段 + inline_declared 标记
            let n = scene.get_mut(root).unwrap();
            n.base_style.taffy_style.display = taffy::Display::None;
            n.base_style.display_mode = crate::style::resolved::DisplayMode::None;
            n.base_style.inline_declared |= INLINE_DISPLAY;
        }
        push_global(&mut scene, rule(".r", "display", "flex"));
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            scene.get(root).unwrap().style.taffy_style.display,
            taffy::Display::None,
            "inline display:none 胜过 class display:flex"
        );
        assert_eq!(
            scene.get(root).unwrap().style.display_mode,
            crate::style::resolved::DisplayMode::None,
            "display_mode 同步 None"
        );
    }

    #[test]
    fn child_explicit_color_not_overridden_by_parent_inline() {
        // 父 inline color red；child 自身 dynamic rule color blue（child inh bit 设）
        // → propagate 不覆盖 child，child 仍 blue。验证 set_map 阻断继承。
        let (mut scene, root, child) = build_parent_child();
        scene.get_mut(child).unwrap().classes = vec!["c".to_string()];
        push_global(&mut scene, rule(".c", "color", "#0000ff"));
        crate::scene::dynamic::set_inline_override(&mut scene, root, "color:#ff0000").unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            scene.get(child).unwrap().style.color,
            [0.0, 0.0, 1.0, 1.0],
            "child 显式声明 blue 不被父 inline red 覆盖"
        );
    }

    #[test]
    fn inline_override_two_level_inheritance() {
        // root inline color red → mid → leaf（均无自身 color）→ leaf 应继承 red。
        let mut root = Node::default();
        root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mid = Node::default();
        let mut leaf = Node::default();
        leaf.kind = NodeKind::TextNode;
        let mut scene = Scene::from_nodes(vec![root, mid, leaf], vec![(0, 1), (1, 2)]);
        let root_id = scene.roots[0];
        let mid_id = scene.get(root_id).unwrap().children[0];
        let leaf_id = scene.get(mid_id).unwrap().children[0];
        crate::scene::dynamic::set_inline_override(&mut scene, root_id, "color:#ff0000").unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            scene.get(leaf_id).unwrap().style.color,
            [1.0, 0.0, 0.0, 1.0],
            "leaf 跨级继承父 inline red"
        );
    }

    #[test]
    fn set_inline_override_ignores_unsupported_prop_no_ghost() {
        // transform 不在 inline_bit 表：完全不写 inline_override（无 ghost state）。
        // padding-top 现在在表（复用 INLINE_PADDING bit）→ 正常置 bit + 生效。
        // 修复前（padding-top 无 bit）：apply_decl 写 taffy_style.padding.top 字段但不置 bit
        // → 下帧 rematch apply_inline_override 不拷该字段 → 静默丢失（ghost state）。
        let (mut scene, root) = build_simple_tree();
        crate::scene::dynamic::set_inline_override(
            &mut scene,
            root,
            "transform: scale(2); padding-top: 10px",
        )
        .unwrap();
        let n = scene.get(root).unwrap();
        // padding-top 有 bit（INLINE_PADDING），transform 无 bit
        assert_ne!(n.inline_set.0, 0, "padding-top 置 bit");
        assert_eq!(
            n.inline_set.0 & INLINE_PADDING,
            INLINE_PADDING,
            "padding-top 复用 INLINE_PADDING bit"
        );
        // padding-top 写进 inline_override（有 bit → 有效）
        use taffy::style::LengthPercentage;
        assert_eq!(
            n.inline_override.taffy_style.padding.top,
            LengthPercentage::length(10.0),
            "padding-top 写入 inline_override"
        );
        // transform 无 bit → 不写 inline_override（transform 字段保持默认，无 ghost）
        // rematch 不 panic
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        let n = scene.get(root).unwrap();
        assert_eq!(
            n.style.taffy_style.padding.top,
            LengthPercentage::length(10.0),
            "rematch 后 padding-top 生效（10px）"
        );
        // 对照：set "width:100px"（支持）→ bit 置 + 生效
        crate::scene::dynamic::set_inline_override(&mut scene, root, "width:100px").unwrap();
        let bit_after = scene.get(root).unwrap().inline_set.0;
        assert_ne!(bit_after, 0, "supported prop（width）置 bit");
        assert_eq!(
            bit_after & INLINE_WIDTH,
            INLINE_WIDTH,
            "INLINE_WIDTH bit 被置"
        );
    }

    #[test]
    fn set_inline_override_no_ghost_when_shorthand_then_longhand() {
        // ghost-state 复现测：set "border:1px solid red"（border 不在 inline_bit 表）→
        // 修复前：apply_decl 写 taffy_style.border + border_color=red 字段但不置 bit。
        // 再 set "border-width:2px"（在表）→ 置 INLINE_BORDER_WIDTH bit。
        // rematch 时 apply_inline_override 拷 inline.taffy_style.border=2px，但
        // border_color 无 bit → 不拷 → style.border_color 用 base 值（None）。但
        // inline_override.border_color 留 red ghost 值污染字段。
        //
        // 修复后：border 整步被跳过（不写字段），只有 border-width 写入；
        // inline_override.border_color 保持默认 None，无 ghost。
        let (mut scene, root) = build_simple_tree();
        crate::scene::dynamic::set_inline_override(&mut scene, root, "border:1px solid red")
            .unwrap();
        // border 不在 inline_bit → 整步跳过，border_color 字段也不写
        assert_eq!(
            scene.get(root).unwrap().inline_set.0,
            0,
            "border shorthand 不置 bit"
        );
        assert_eq!(
            scene.get(root).unwrap().inline_override.border_color,
            None,
            "border shorthand 不写 border_color 字段（无 ghost）"
        );
        // 再 set border-width（在表）
        crate::scene::dynamic::set_inline_override(&mut scene, root, "border-width:2px").unwrap();
        let n = scene.get(root).unwrap();
        assert_ne!(
            n.inline_set.0 & INLINE_BORDER_WIDTH,
            0,
            "border-width 置 bit"
        );
        // border_color 字段仍未被污染（默认 None，未因 border shorthand 的 red 残留）
        assert_eq!(
            n.inline_override.border_color, None,
            "border_color 无 ghost red 残留"
        );
        // rematch：border 2px 生效（bit 置），border_color 仍 None（base 值）
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        use taffy::style::LengthPercentage;
        let s = &scene.get(root).unwrap().style;
        assert_eq!(
            s.taffy_style.border.top,
            LengthPercentage::length(2.0),
            "border-width 2px inline 生效"
        );
        assert_eq!(
            s.border_color, None,
            "border_color 用 base 值（无 ghost red）"
        );
    }

    /// CSS 作用域隔离回归测试：页面根(SCOPE_ROOT) → child → 组件实例根(SCOPE_ROOT)
    /// → leaf(.leaf)。页面根作用域的 .leaf 规则不应命中实例内部节点——leaf 的 node_scope =
    /// 实例根（沿父链最近的 SCOPE_ROOT）≠ 页面根，scoped 规则被过滤。
    ///
    /// 此测试锁定 SCOPE_ROOT/LOOKUP_SCOPE 拆分后 CSS 作用域隔离语义不变（SCOPE_ROOT 仍是
    /// 作用域隔离的唯一依据）。注意：实例根在此测同样打双 flag（复现生产 instantiate 路径），
    /// 但即便只打 SCOPE_ROOT，leaf 的 node_scope 仍是实例根，隔离仍成立。
    #[test]
    fn page_scoped_rule_does_not_match_component_instance_node() {
        let mut page_root = Node::default();
        page_root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 400.0,
        };
        let mut child = Node::default();
        child.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut instance_root = Node::default();
        instance_root.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let mut leaf = Node::default();
        leaf.classes = vec!["leaf".to_string()];
        leaf.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 50.0,
        };
        // 树：page_root → child → instance_root → leaf
        let mut s = Scene::from_nodes(
            vec![page_root, child, instance_root, leaf],
            vec![(0, 1), (1, 2), (2, 3)],
        );
        let page_root_id = s.roots[0];
        let child_id = s.get(page_root_id).unwrap().children[0];
        let instance_root_id = s.get(child_id).unwrap().children[0];
        let leaf_id = s.get(instance_root_id).unwrap().children[0];
        // 页面根 + 实例根都打 SCOPE_ROOT（复现 create_root / instantiate 生产路径）；
        // 页面根同时打 LOOKUP_SCOPE（lookup 边界，此测不验证 lookup，只验证 CSS 隔离）。
        s.get_mut(page_root_id)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::SCOPE_ROOT | NodeFlags::LOOKUP_SCOPE);
        s.get_mut(instance_root_id)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::SCOPE_ROOT | NodeFlags::LOOKUP_SCOPE);
        // 页面根作用域的 .leaf 规则：叶子 base color 为默认（非红）。若作用域隔离失效，
        // 该规则会穿透命中 leaf 染红。
        let base_color = s.get(leaf_id).unwrap().base_style.color;
        push_scoped(&mut s, page_root_id, rule(".leaf", "color", "#ff0000"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(leaf_id).unwrap().style.color,
            base_color,
            "页面根作用域的 .leaf 规则不应命中实例内节点（作用域隔离：leaf scope = 实例根 ≠ 页面根）"
        );
    }

    /// 构造 TabList + N 个 role=tab 子节点的 scene（复刻 TabList 实例化形态）。
    /// 返回 (scene, tablist_id, [tab_id,...])。
    fn tablist_scene(num_tabs: usize, selected_index: usize) -> (Scene, NodeId, Vec<NodeId>) {
        let mut tl = Node::default();
        tl.kind = NodeKind::TabList;
        let mut tabs = Vec::with_capacity(num_tabs);
        for _ in 0..num_tabs {
            let mut t = Node::default();
            t.kind = NodeKind::Tab;
            tabs.push(t);
        }
        let mut nodes = vec![tl];
        nodes.extend(tabs);
        let edges: Vec<(usize, usize)> = (0..num_tabs).map(|i| (0, i + 1)).collect();
        let mut s = Scene::from_nodes(nodes, edges);
        let tl_id = s.roots[0];
        let tab_ids: Vec<NodeId> = s.get(tl_id).unwrap().children.clone();
        for &tid in &tab_ids {
            s.roles.insert(
                tid,
                RoleInfo {
                    role: Some("tab".into()),
                    ..Default::default()
                },
            );
        }
        s.controls.ensure(
            tl_id,
            ControlState::TabList {
                selected_index,
                manual_activation: false,
            },
        );
        (s, tl_id, tab_ids)
    }

    #[test]
    fn tab_aria_selected_synth_from_parent_tablist() {
        // TabList(selected_index=1) + 2 个 Tab 子。t0=false、t1=true、tablist=None（非 Tab）。
        let (s, tl, tabs) = tablist_scene(2, 1);
        let t0 = tabs[0];
        let t1 = tabs[1];
        assert_eq!(
            synth_aria_value(&s, t0, "selected"),
            Some("false".into()),
            "t0 不是激活 tab → aria-selected=false"
        );
        assert_eq!(
            synth_aria_value(&s, t1, "selected"),
            Some("true".into()),
            "t1 是激活 tab → aria-selected=true"
        );
        assert_eq!(
            synth_aria_value(&s, tl, "selected"),
            None,
            "TabList 非 Tab → aria-selected 无语义"
        );
    }

    #[test]
    fn tab_aria_selected_no_tablist_ancestor_returns_none() {
        // Tab 无 TabList 父（孤立 Tab）→ aria-selected 无语义。
        let s = Scene::from_nodes(vec![test_node(NodeKind::Tab)], vec![]);
        let id = s.roots[0];
        assert_eq!(
            synth_aria_value(&s, id, "selected"),
            None,
            "无 TabList 父的孤立 Tab → aria-selected 无语义"
        );
    }

    #[test]
    fn tab_aria_selected_parent_not_tablist_returns_none() {
        // Tab 的父是普通 Container（不是 TabList）→ 向上找不到 TabList → None。
        let mut parent = Node::default();
        parent.kind = NodeKind::Container;
        let tab = test_node(NodeKind::Tab);
        let mut s = Scene::from_nodes(vec![parent, tab], vec![(0, 1)]);
        let tab_id = s.get(s.roots[0]).unwrap().children[0];
        s.roles.insert(
            tab_id,
            RoleInfo {
                role: Some("tab".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            synth_aria_value(&s, tab_id, "selected"),
            None,
            "Tab 父非 TabList → aria-selected 无语义"
        );
    }

    #[test]
    fn attr_selector_aria_selected_hits_active_tab() {
        // [aria-selected="true"] 命中激活 tab（t1），不命中非激活 tab（t0）。
        let (s, _tl, tabs) = tablist_scene(2, 1);
        let t0 = tabs[0];
        let t1 = tabs[1];
        let sel_true = hand_selector(r#"[aria-selected="true"]"#);
        let sel_false = hand_selector(r#"[aria-selected="false"]"#);
        assert!(
            !compound_matches_node(&sel_true.compound[0], t0, &s),
            "t0 → [aria-selected=\"true\"] 不命中"
        );
        assert!(
            compound_matches_node(&sel_true.compound[0], t1, &s),
            "t1 → [aria-selected=\"true\"] 命中"
        );
        // 反向：[aria-selected="false"] 命中 t0，不命中 t1
        assert!(
            compound_matches_node(&sel_false.compound[0], t0, &s),
            "t0 → [aria-selected=\"false\"] 命中"
        );
        assert!(
            !compound_matches_node(&sel_false.compound[0], t1, &s),
            "t1 → [aria-selected=\"false\"] 不命中"
        );
    }

    #[test]
    fn attr_selector_aria_selected_exists_matches_tab() {
        // [aria-selected] 存在形式：Tab 节点合成值非 None → Exists 命中；非 Tab → None → 不命中。
        let (s, tl, tabs) = tablist_scene(2, 0);
        let t0 = tabs[0];
        let sel = hand_selector("[aria-selected]");
        assert!(
            compound_matches_node(&sel.compound[0], t0, &s),
            "Tab → [aria-selected] Exists 命中"
        );
        assert!(
            !compound_matches_node(&sel.compound[0], tl, &s),
            "TabList → [aria-selected] Exists 不命中"
        );
    }

    #[test]
    fn inline_override_z_index_survives_rematch_and_unset() {
        // 便签层 z-index：set → rematch 应用进 live style；unset → 回落 base（默认 0）。
        // 验证 u64 位图 bit 32 不被截断（u32 位图时代该 bit 装不下）。
        let (mut scene, root, _child) = build_parent_child();
        crate::scene::dynamic::set_inline_override(&mut scene, root, "z-index:7").unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(scene.get(root).unwrap().style.z_index, 7);
        crate::scene::dynamic::unset_inline_override(&mut scene, root, "z-index").unwrap();
        rematch_pseudo_classes(&mut scene, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            scene.get(root).unwrap().style.z_index,
            0,
            "unset 后回落 base_style 默认 0"
        );
    }

    // —— #10 layout / box-shadow transition 通道 ——

    #[test]
    fn rematch_emits_width_transition_same_domain() {
        // .btn:hover height 100px→0px 同域 → Width/Height 请求，载荷 [value, domain_code]。
        // base 须显式声明（auto 端不建 tween——双端齐全才有动画语义）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        {
            let n = s.get_mut(bid).unwrap();
            use crate::style::mapping::apply_decl;
            assert!(apply_decl(&mut n.base_style, "height", "100px"));
            n.base_style.transition = vec![TransitionSpec {
                prop: Some(TweenProp::Height),
                duration: 0.3,
                ease: Ease::Linear,
                delay: 0.0,
            }];
            n.interaction
                .flags
                .insert(crate::scene::node::NodeFlags::CASCALED);
            // 首帧级联产物（old_style 读 n.style 非 base_style）
            n.style = n.base_style.clone();
        }
        push_global(&mut s, rule(".btn:hover", "height", "0px"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        s.pending_transitions.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.pending_transitions.len(), 1, "同域 height 变化 → 1 请求");
        let r = &s.pending_transitions[0];
        assert!(matches!(r.prop, TweenProp::Height));
        assert_eq!(
            r.end[1] as u32,
            crate::scene::LenDomain::Px as u32,
            "载荷第 2 槽 = 域码"
        );
    }

    #[test]
    fn rematch_cross_domain_width_snaps_with_warning() {
        // 端点跨域（默认 auto base 100px? —— btn_scene 无显式 height，auto 端 = 不建 tween）；
        // 这里构造 px→% 跨域：base 显式 100px + hover 50%。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        {
            let n = s.get_mut(bid).unwrap();
            use crate::style::mapping::apply_decl;
            assert!(apply_decl(&mut n.base_style, "height", "100px"));
            n.base_style.transition = vec![TransitionSpec {
                prop: Some(TweenProp::Height),
                duration: 0.3,
                ease: Ease::Linear,
                delay: 0.0,
            }];
            n.interaction
                .flags
                .insert(crate::scene::node::NodeFlags::CASCALED);
            // 首帧级联产物（old_style 读 n.style 非 base_style）
            n.style = n.base_style.clone();
        }
        push_global(&mut s, rule(".btn:hover", "height", "50%"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        s.pending_transitions.clear();
        s.pending_anim_warnings.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert!(
            s.pending_transitions.is_empty(),
            "跨域端点不建 tween（新级联值直接生效 = snap）"
        );
        assert_eq!(s.pending_anim_warnings.len(), 1, "snap 警告事件入队");
        assert_eq!(
            s.pending_anim_warnings[0].event_type,
            crate::input::EVT_TRANSITION_SNAP
        );
        assert_eq!(
            s.pending_anim_warnings[0].click_count,
            TweenProp::Height as u8
        );
    }

    #[test]
    fn rematch_auto_endpoint_no_tween_no_warning() {
        // auto 端点（base 未声明 height → auto）：不建 tween 也不警告（CSS 语义：
        // auto→显式值的变化 rematch 已写新值 = 直接跳变；警告只留给跨域——auto 端点
        // 与显式端点的组合在静态视野已被围栏拦，运行时 auto base 是常见常态
        // （未声明 = auto），逐次警告太吵）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        s.get_mut(bid).unwrap().base_style.transition = vec![TransitionSpec {
            prop: Some(TweenProp::Height),
            duration: 0.3,
            ease: Ease::Linear,
            delay: 0.0,
        }];
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(crate::scene::node::NodeFlags::CASCALED);
        push_global(&mut s, rule(".btn:hover", "height", "200px"));
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        s.pending_transitions.clear();
        s.pending_anim_warnings.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert!(s.pending_transitions.is_empty(), "auto 端不建 tween");
        assert!(s.pending_anim_warnings.is_empty(), "auto 端不产跨域警告");
    }

    #[test]
    fn rematch_emits_box_shadow_transition_with_payload() {
        // .btn:hover 换 box-shadow + transition:box-shadow → BoxShadow 请求 + ShadowPair 载荷。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        s.get_mut(bid).unwrap().base_style.transition = vec![TransitionSpec {
            prop: Some(TweenProp::BoxShadow),
            duration: 0.3,
            ease: Ease::Linear,
            delay: 0.0,
        }];
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(crate::scene::node::NodeFlags::CASCALED);
        push_global(
            &mut s,
            rule(".btn:hover", "box-shadow", "0 8px 16px rgba(0,0,0,0.5)"),
        );
        s.get_mut(bid)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::HOVERED);
        s.pending_transitions.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.pending_transitions.len(), 1);
        let r = &s.pending_transitions[0];
        assert!(matches!(r.prop, TweenProp::BoxShadow));
        let pair = r.shadow.as_ref().expect("列表载荷在 shadow 字段");
        assert!(
            pair.start.is_empty(),
            "base 无阴影 → 空列表起点（透明淡入）"
        );
        assert_eq!(pair.end.len(), 1);
        assert!((pair.end[0].oy - 8.0).abs() < 1e-5);
    }

    // ===== #11 运行时 CSS 注入 + custom props/var() =====

    /// 多声明规则构造（rule() 只收单声明）。
    fn rule_decls(sel: &str, decls: &[(&str, &str)]) -> DynamicRule {
        DynamicRule {
            selector: hand_selector(sel),
            declarations: decls
                .iter()
                .map(|(p, v)| Declaration {
                    prop: p.to_string(),
                    value: v.to_string(),
                })
                .collect(),
        }
    }

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

    #[test]
    fn runtime_rule_add_applies_immediately_next_rematch() {
        // Add 注入 → 下一次 rematch 即生效（每帧全量，无 dirty 机制）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        let set_id =
            crate::scene::dynamic::style_sheet_add(&mut s, vec![rule(".btn", "color", "#ff0000")]);
        assert!(set_id > 0);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, RED);
    }

    /// 运行时 CSS 通道收下 clip-path（#52）：Add 类规则 → 下一次 rematch 生效
    /// （apply_decl 同一 arm；消费点惰性派生，无需 solve 存储字段）。
    #[test]
    fn runtime_rule_clip_path_applies_on_rematch() {
        use crate::style::resolved::{ClipPathDecl, ClipShape};
        let mut s = btn_scene();
        let bid = btn_id(&s);
        crate::scene::dynamic::style_sheet_add(
            &mut s,
            vec![rule(".btn", "clip-path", "circle(50%)")],
        );
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        let style = &s.get(bid).unwrap().style;
        assert!(matches!(style.clip_path, Some(ClipPathDecl::Circle { .. })));
        // 惰性派生：按 layout 100×100 解析成内切圆（消费点同款调用）。
        let shape = style.clip_path.as_ref().unwrap().resolve(100.0, 100.0);
        match shape {
            ClipShape::Circle { r, .. } => assert!((r - 50.0).abs() < 1e-4),
            _ => panic!(),
        }
        // none 清除（toggle 场景）。
        crate::scene::dynamic::style_sheet_add(&mut s, vec![rule(".btn", "clip-path", "none")]);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert!(s.get(bid).unwrap().style.clip_path.is_none(), "none 清除");
    }

    #[test]
    fn runtime_rule_same_specificity_later_add_wins() {
        // 「与模板 CSS 同 cascade 优先级」+ 追加序 = 文档序：同 specificity 后 Add 赢。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn", "color", "#ff0000"));
        crate::scene::dynamic::style_sheet_add(&mut s, vec![rule(".btn", "color", "#0000ff")]);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, BLUE, "后 Add 赢");
    }

    #[test]
    fn runtime_rule_remove_restores_and_clear_empties() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        let set1 =
            crate::scene::dynamic::style_sheet_add(&mut s, vec![rule(".btn", "color", "#ff0000")]);
        let set2 =
            crate::scene::dynamic::style_sheet_add(&mut s, vec![rule(".btn", "color", "#0000ff")]);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, BLUE);
        // Dispose set2（后注入的胜出者）→ 回落 set1 红。
        assert!(crate::scene::dynamic::style_sheet_remove(&mut s, set2));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, RED);
        // 重复 Dispose → false；Clear 清空运行时集。
        assert!(!crate::scene::dynamic::style_sheet_remove(&mut s, set2));
        crate::scene::dynamic::style_sheet_clear(&mut s);
        assert!(s.runtime_rule_sets.is_empty());
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_ne!(s.get(bid).unwrap().style.color, RED);
        let _ = set1;
    }

    #[test]
    fn custom_props_cross_scope_boundary_inheritance() {
        // main-design §279 契约锁：--* 跨组件（作用域）边界继承——scope 根规则声明的
        // --accent 对边界内后代可见（var 环境沿父链走查不因 SCOPE_ROOT 停步——与
        // 后代选择器的墙语义不同，custom props 的墙是透的）。
        let mut s = btn_scene();
        let root_id = s.roots[0];
        let bid = btn_id(&s);
        s.get_mut(root_id)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::SCOPE_ROOT);
        push_global(&mut s, rule("div", "--accent", "#ff0000"));
        push_global(&mut s, rule(".btn", "color", "var(--accent)"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.color,
            RED,
            "custom prop 跨作用域边界继承给边界内后代"
        );
    }

    #[test]
    fn runtime_rule_hits_inside_scoped_subtree() {
        // 穿墙语义：全局注入规则可及 scoped（组件展开域）内部节点——打包期内容墙
        // 不约束运行时注入（public-api §10.2）。root 打 SCOPE_ROOT，运行时全局规则
        // 仍命中其子。
        let mut s = btn_scene();
        let root_id = s.roots[0];
        let bid = btn_id(&s);
        s.get_mut(root_id)
            .unwrap()
            .interaction
            .flags
            .insert(NodeFlags::SCOPE_ROOT);
        crate::scene::dynamic::style_sheet_add(&mut s, vec![rule(".btn", "color", "#0000ff")]);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.color,
            BLUE,
            "全局注入规则跨作用域命中"
        );
    }

    /// var() 代换 clip-path（#52 涌现能力实锤）：规则声明 --sm-mask + 值消费
    /// `clip-path: var(--sm-mask)`，SetVar 换值后 rematch 重放 → shape 翻转。
    #[test]
    fn var_substitution_drives_clip_path() {
        use crate::style::resolved::{ClipPathDecl, ClipShape};
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(
            &mut s,
            rule_decls(
                ".btn",
                &[
                    ("--sm-mask", "circle(50%)"),
                    ("clip-path", "var(--sm-mask)"),
                ],
            ),
        );
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert!(
            matches!(
                s.get(bid).unwrap().style.clip_path,
                Some(ClipPathDecl::Circle { .. })
            ),
            "var() 代换 circle 生效"
        );
        // SetVar 换 polygon → rematch 后 shape 翻转（运行时换遮罩通道）。
        crate::scene::dynamic::node_set_var(
            &mut s,
            bid,
            "--sm-mask",
            "polygon(50% 0%, 100% 50%, 50% 100%, 0% 50%)",
        )
        .expect("set_var");
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert!(
            matches!(
                s.get(bid).unwrap().style.clip_path,
                Some(ClipPathDecl::Polygon { .. })
            ),
            "SetVar 换形后 polygon 生效"
        );
        // 惰性派生链路完整：多边形几何按 100×100 解析。
        let shape = s
            .get(bid)
            .unwrap()
            .style
            .clip_path
            .as_ref()
            .unwrap()
            .resolve(100.0, 100.0);
        match shape {
            ClipShape::Polygon { points } => assert_eq!(points.len(), 4),
            _ => panic!("polygon 形错"),
        }
    }

    #[test]
    fn var_consumes_rule_declared_custom_prop() {
        // .btn 同规则集里声明 --accent + color: var(--accent) → 消费生效。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(
            &mut s,
            rule_decls(
                ".btn",
                &[("--accent", "#ff0000"), ("color", "var(--accent)")],
            ),
        );
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, RED);
    }

    #[test]
    fn var_inherits_down_tree_from_ancestor_rule() {
        // root 规则声明 --accent；子消费 var(--accent) 继承解析（main-design：--* 跨边界传递）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule("div", "--accent", "#ff0000"));
        push_global(&mut s, rule(".btn", "color", "var(--accent)"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.color,
            RED,
            "子从父继承 custom prop"
        );
        // SetVar 在子节点覆盖继承值
        crate::scene::dynamic::node_set_var(&mut s, bid, "--accent", "#0000ff").unwrap();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, BLUE, "SetVar 最高优先级");
        // RemoveVar 回落继承值
        crate::scene::dynamic::node_remove_var(&mut s, bid, "--accent").unwrap();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, RED, "RemoveVar 回落");
    }

    #[test]
    fn nested_chain_resolves_through_declaration_site() {
        // --a: var(--b) 与 --b 同在 root 声明 → --a 解析为 --b 值；子消费继承解析结果。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(
            &mut s,
            rule_decls("div", &[("--b", "#ff0000"), ("--a", "var(--b)")]),
        );
        push_global(&mut s, rule(".btn", "color", "var(--a)"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, RED, "嵌套链解析");
    }

    #[test]
    fn var_missing_without_fallback_skips_declaration_with_warning() {
        // 缺失且无 fallback：声明跳过（color 保持 base 默认）+ warn-once 进 warnings。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn", "color", "var(--missing)"));
        s.warnings.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_ne!(s.get(bid).unwrap().style.color, RED, "声明被跳过");
        assert!(
            s.warnings.iter().any(|w| w.contains("--missing")),
            "warn-once 落 warnings: {:?}",
            s.warnings
        );
    }

    #[test]
    fn var_fallback_literal_applies() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn", "color", "var(--missing, #0000ff)"));
        s.warnings.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, BLUE, "fallback 字面量生效");
        assert!(
            s.warnings.iter().all(|w| !w.contains("--missing")),
            "有 fallback 不告警: {:?}",
            s.warnings
        );
    }

    #[test]
    fn var_cycle_makes_declaration_invalid_but_fallback_works() {
        // --a ↔ --b 互指：环上属性 invalid → color: var(--a) 跳过、无 fallback 的背景也跳过；
        // 带 fallback 的消费走 fallback。环进 warnings（运行时分层 fail-loud：不抛异常、留诊断）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(
            &mut s,
            rule_decls("div", &[("--a", "var(--b)"), ("--b", "var(--a)")]),
        );
        push_global(&mut s, rule(".btn", "color", "var(--a)"));
        push_global(
            &mut s,
            rule(".btn", "background-color", "var(--a, #0000ff)"),
        );
        s.warnings.clear();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        let st = &s.get(bid).unwrap().style;
        assert_ne!(st.color, RED, "环 → color 声明跳过");
        assert_eq!(
            st.background_color,
            Some(BLUE),
            "环 + fallback → fallback 生效"
        );
        assert!(
            s.warnings.iter().any(|w| w.contains("cycle")),
            "环 warn-once: {:?}",
            s.warnings
        );
    }

    #[test]
    fn var_layer_off_path_unchanged_for_plain_rules() {
        // 无任何 var/custom 源 → 单阶段原路径（行为回归锚：普通规则照旧生效）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn", "color", "#ff0000"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, RED);
        assert!(s.node_vars.is_empty() && s.runtime_rule_sets.is_empty());
    }

    #[test]
    fn deferred_inline_var_consumption_at_inline_priority() {
        // 行内 var 消费（base_style.deferred_inline 携带 color: var(--accent)）：
        // ① 值来自规则声明的 --accent；② inline 优先级——同 prop 的 class 规则不覆盖
        //（css_resolve 打包期已烙 inline_declared 位，此处手工模拟完整烘焙面）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule("div", "--accent", "#ff0000"));
        push_global(&mut s, rule(".btn", "color", "#00ff00"));
        {
            let n = s.get_mut(bid).unwrap();
            n.base_style.deferred_inline.push(Declaration {
                prop: "color".to_string(),
                value: "var(--accent)".to_string(),
            });
            n.base_style.inline_declared |= crate::style::dynamic::INH_COLOR;
        }
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(
            s.get(bid).unwrap().style.color,
            RED,
            "行内 var 消费赢过 class 规则字面值"
        );
    }

    #[test]
    fn inline_custom_overrides_rule_custom_and_setvar_beats_both() {
        // 优先级链：规则 --x < 行内 --x（deferred_inline）< SetVar。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        push_global(&mut s, rule(".btn", "--x", "#ff0000"));
        push_global(&mut s, rule(".btn", "color", "var(--x)"));
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, RED, "规则声明基线");
        // 行内 --x 覆盖
        s.get_mut(bid)
            .unwrap()
            .base_style
            .deferred_inline
            .push(Declaration {
                prop: "--x".to_string(),
                value: "#00ff00".to_string(),
            });
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, GREEN, "行内覆盖规则");
        // SetVar 最高
        crate::scene::dynamic::node_set_var(&mut s, bid, "--x", "#0000ff").unwrap();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, BLUE, "SetVar 最高");
        // RemoveVar 回落行内值（不是规则值——SetVar 只撤自己）
        crate::scene::dynamic::node_remove_var(&mut s, bid, "--x").unwrap();
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, GREEN, "回落行内");
    }

    #[test]
    fn runtime_injected_rule_can_declare_custom_props() {
        // Add 通道注入 --x 声明 + 消费规则（主题热切换的最小闭环）。
        let mut s = btn_scene();
        let bid = btn_id(&s);
        let set_a = crate::scene::dynamic::style_sheet_add(
            &mut s,
            vec![rule_decls("div", &[("--accent", "#ff0000")])],
        );
        crate::scene::dynamic::style_sheet_add(
            &mut s,
            vec![rule(".btn", "color", "var(--accent)")],
        );
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_eq!(s.get(bid).unwrap().style.color, RED);
        // Dispose 声明集 → --accent 消失 → 消费声明跳过（颜色回默认）
        crate::scene::dynamic::style_sheet_remove(&mut s, set_a);
        rematch_pseudo_classes(&mut s, (1080.0, 1920.0), [0.0; 4]);
        assert_ne!(s.get(bid).unwrap().style.color, RED);
    }

    #[test]
    fn setvar_rejects_non_custom_name_and_dead_node() {
        let mut s = btn_scene();
        let bid = btn_id(&s);
        assert!(crate::scene::dynamic::node_set_var(&mut s, bid, "accent", "#fff").is_err());
        assert!(crate::scene::dynamic::node_set_var(&mut s, bid, "--", "#fff").is_err());
        let dead = NodeId(999_999);
        assert!(crate::scene::dynamic::node_set_var(&mut s, dead, "--x", "1").is_err());
        assert!(crate::scene::dynamic::node_remove_var(&mut s, dead, "--x").is_err());
    }
}
