//! Layout 层：taffy 集成。
//!
//! 消费 `Scene`（Node 树 + `ResolvedStyle`），建 taffy 树，注册叶子节点的
//! 测量上下文（Text/Image），solve 后把 taffy 的 `Layout.location`/`size`
//! 回写进 `Node.layout_rect`/`clip_rect`。
//!
//! # taffy 0.14 测量契约
//!
//! - `TaffyTree<NodeContext>`：节点上下文是泛型，叶子节点用
//!   `new_leaf_with_context(style, ctx)` 存一个 owned `NodeContext`。
//! - 单个 `compute_layout_with_measure(root, avail, FnMut(LayoutInput, NodeId,
//!   Option<&mut NodeContext>, &Style) -> LayoutOutput)` 闭包负责一切**无 children
//!   节点**的布局——不只 context 节点（0.12 期只调 context 节点）。闭包即叶子布局
//!   算法：padding/border/min/max/box-sizing 合成自理，故各分支委托
//!   `taffy::compute_leaf_layout`（= 0.12 期 taffy 内部叶子路径），内层闭包只测
//!   content 尺寸（Image 分支返回 outer size——ctx 无 padding，契约等价）。
//! - 内层测量契约：ComputeSize 轮 known 原样是 border-box；avail 的 Definite 已被
//!   `compute_leaf_layout` 扣过 content_box_inset（content 域）——见 solve 闭包内注释。
//!
//! 测量是单个 `FnMut`（非 'static），生命周期与 `compute_layout_with_measure`
//! 调用同界——闭包内借 `fonts: &FontTable` 合法。每个叶子的文本参数（content/font_size +
//! family 等）已 owned 进 `NodeContext::Text`（不含 Font 实例），font 在闭包内按 family
//! 查 FontTable 取得。`solve` 签名收 `fonts: &FontTable`（不破下游 stage 契约）。
//!
//! taffy 的 `Style` 无 `order`，不做 flex order 排序（render 层按 DOM 顺序 /
//! layout 输出的 `Layout.order` 渲染）。
//!
//! 核心知图尺寸（打包期 PNG IHDR 静态，Stage 持 path→(w,h) 尺寸表）+ 不知图集
//! （运行时纹理/UV 归 Unity）。solve 接 `image_sizes: &HashMap<String,(u32,u32)>` 查 Image intrinsic
//! 尺寸（三档：CSS > 真实像素 > 64×64）。render payload 带 path，UV 全图 (0,0)-(1,1)。

use crate::scene::node::{is_whitespace_only_text, NodeId, NodeKind, Rect, Scene};
use crate::style::resolved::{OverflowMode, TextAlign};
use crate::text::layout::{measure_text, FontTable, TextLayout};
use std::collections::HashMap;
use taffy::prelude::*;
use taffy::tree::{LayoutInput, LayoutOutput};

/// 图尺寸表类型别名：归一化 path → (w, h) 像素（打包期 PNG IHDR 静态）。
/// `solve`/`build_render_nodes` 接 `&HashMap<String, (u32, u32)>` 查 Image intrinsic 尺寸。
pub type ImageSizeTable = HashMap<String, (u32, u32)>;

/// Yio OverflowMode → taffy Overflow（Auto→Scroll，taffy 无 Auto 变体）。
/// Hidden/Scroll 让 taffy flex automatic min-size=0（CSS flex §4.5，taffy style/mod.rs:124）——
/// 容器不被 content min-content 撑开，content 可溢出 scroll。不设则 taffy 默认 Visible →
/// 容器被 content 撑开（viewport=content）→ overlap=0 → scroll 失效。
fn map_overflow(m: OverflowMode) -> taffy::style::Overflow {
    match m {
        OverflowMode::Visible => taffy::style::Overflow::Visible,
        OverflowMode::Hidden => taffy::style::Overflow::Hidden,
        OverflowMode::Scroll => taffy::style::Overflow::Scroll,
        OverflowMode::Auto => taffy::style::Overflow::Scroll,
    }
}

/// taffy LengthPercentage → f32（固定尺寸节点 Percent 罕见，按 0 处理）。
fn lp(v: taffy::style::LengthPercentage) -> f32 {
    // taffy 0.12：LengthPercentage 是 pub struct(CompactLength) tagged pointer，
    // 内字段私有无法 match 变体——用 into_raw + tag 解构（只要 Length 分支）。
    let cl = v.into_raw();
    if cl.tag() == taffy::style::CompactLength::LENGTH_TAG {
        cl.value()
    } else {
        0.0
    }
}

/// 叶子节点的测量上下文。Container/Button 无上下文（用 None 叶子或 new_with_children）。
/// PartialEq 供增量 solve 的变更检测（ctx 变 → set_node_context 标脏重测）；
/// Debug/Clone 供 Scene 的 layout_cache 跨帧持有。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MeasureContext {
    /// Text 叶子：存全部测量参数（content owned）+ 字体度量字段 + 字体族。
    /// font 实例 *不* 进 context——调用方在测量闭包中按 family 查 FontTable 取 Font。
    Text {
        content: String,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        align: TextAlign,
        /// 换行控制（#73）。静态文本 = style.wrap_control()；文本控件 =
        /// control_wrap_control（空白冻结 pre 系，保光标字节映射）。
        wrap: crate::text::layout::WrapControl,
        /// 节点的 font_family。None 表示用 FontTable 的 default。
        family: Option<String>,
        /// 节点 style.color（plain text 整段同色；进 GlyphRun.color 供 build per-vertex）。
        color: [f32; 4],
        /// 节点 style.font_weight（≥700 → Bold，经 weight_from_font_weight 转 RichWeight 进 GlyphRun.weight）。
        font_weight: u16,
        /// 水平 padding+border 总 inset（左+右）。taffy 传 known.width = 节点 border-box 宽；
        /// 文字须在 content area（known - inset）内换行 + 对齐，否则吃到 padding 超框。
        h_inset: f32,
    },
    /// RichText 叶子（v1.7）：inline flow 封装在 measure_rich_text。
    /// runs owned（parse 期产的扁平 run 流，含 per-run 样式）。
    /// `align` 传入 measure_rich_text（每行容器内偏移）。
    RichText {
        runs: Vec<crate::text::rich::RichRun>,
        line_height: f32,
        /// CSS letter-spacing（px）。rich inline flow 的 token 宽/glyph 定位均计入。
        letter_spacing: f32,
        align: TextAlign,
        /// 换行控制（#73）：white-space/word-break 等全量进 measure_rich_text
        /// （#73 起真正接线——此前 rich 忽略 nowrap 的洞已补）。
        wrap: crate::text::layout::WrapControl,
        /// 节点的 font_family。None 表示用 FontTable 的 default。
        family: Option<String>,
        /// 水平 padding+border 总 inset（左+右）。同 Text：文字在 content area 内换行/对齐。
        h_inset: f32,
    },
    /// Image 叶子：intrinsic 像素 + css width/height 维度。闭包消费 taffy 的 known 解析
    /// Percent/fit（Percent width taffy 传 known.width=Some(解析宽)，闭包据此等比 height）。
    Image {
        iw: f32,
        ih: f32,
        w_dim: taffy::style::Dimension,
        h_dim: taffy::style::Dimension,
    },
}

/// 跨帧持久化的 taffy 布局树——增量 solve 的载体（`Scene.layout_cache`，坑 186 根治）。
///
/// 每帧 solve 不再重建 taffy 树，做「期望态 diff」：节点 style/measure ctx 变更走
/// `set_style`/`set_node_context`（值比较短路，稳态帧零脏标），结构变更（增删/重排/
/// 重挂/排除项转正）走 `set_children`/`remove`，靠 taffy 自带 dirty 上溯 + 布局缓存
/// 跳过干净子树。
///
/// `ids` 按 NodeId.index() 索引（容量而非存活数——remove 后 slotmap idx 不变，
/// 按存活数分配会越界）。每格存 (scene NodeId, taffy NodeId)：NodeId 含
/// generation，slot 复用后 gen 不符即判死条目，杜绝新节点错配旧 taffy 节点。
#[derive(Debug, Clone)]
pub struct LayoutCache {
    pub(crate) tree: TaffyTree<MeasureContext>,
    pub(crate) ids: Vec<Option<(NodeId, taffy::NodeId)>>,
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self {
            tree: TaffyTree::new(),
            ids: Vec::new(),
        }
    }
}

/// 就地 solve（增量）：持久 taffy 树期望态 diff → compute_layout → 回写
/// layout_rect/clip_rect。
///
/// `root_size` 是根节点固定尺寸（viewport / surface 尺寸）。`fonts` borrows
/// FontTable 到 `compute_layout_with_measure` 结束，闭包内按 family 查字体喂给 `measure_text`。
///
/// `image_sizes` = Stage 持有的 path→(w,h) 尺寸表（打包期 PNG IHDR 静态）。
/// Image measure 查此表算 intrinsic 尺寸（三档：CSS > 真实像素 > 64×64）。
/// path 缺失或 w/h=0 → fallback 64×64（核心不知图集，但知图尺寸）。
/// 动画长度 override → taffy Dimension（#10）。px/pct 落原生 tag；vw/vh/vmin/vmax
/// 按当帧 root_size 换算成 px（与 `ViewportStyle::apply` 同语义，但来源是 anim override）。
fn anim_len_to_dimension(
    l: crate::scene::AnimLen,
    root_size: (f32, f32),
) -> taffy::style::Dimension {
    use crate::scene::LenDomain;
    use taffy::style::Dimension;
    match l.domain {
        LenDomain::Px => Dimension::length(l.value),
        LenDomain::Pct => Dimension::percent(l.value / 100.0),
        LenDomain::Vw => Dimension::length(l.value / 100.0 * root_size.0),
        LenDomain::Vh => Dimension::length(l.value / 100.0 * root_size.1),
        LenDomain::Vmin => Dimension::length(l.value / 100.0 * root_size.0.min(root_size.1)),
        LenDomain::Vmax => Dimension::length(l.value / 100.0 * root_size.0.max(root_size.1)),
    }
}

pub fn solve(
    scene: &mut Scene,
    fonts: &FontTable,
    root_size: (f32, f32),
    safe_insets: [f32; 4],
    image_sizes: &ImageSizeTable,
) {
    // 防御：空 roots（空 scene）无几何可 solve——直接返回，避免 roots[0] 越界 panic。
    // Stage 可能在 scene 未装内容时 tick（如测/边界），不应 panic。
    if scene.roots.is_empty() {
        return;
    }
    // 持久树 mem::take 出 scene（同 text_measure_cache 模式）：sync 借 &Scene +
    // &mut 树/映射两不相扰，末尾整体写回。
    let mut cache = std::mem::take(&mut scene.layout_cache);
    let cap = scene.nodes.capacity() + 1;
    if cache.ids.len() < cap {
        cache.ids.resize(cap, None);
    }
    // 老映射快照（清退依据）：sync 重建 ids，未被新映射保留的老 taffy 节点
    //（scene 节点已死 / slot 复用 gen 不符 / 本帧被排除——纯空白文本、rich 折叠子）
    // 在 walk 后统一 remove。
    let old = std::mem::replace(&mut cache.ids, vec![None; cap]);

    /// 期望态同步（后序 DFS——子先同步，父的期望子列表才有 tid 可挂；规则与全量
    /// build 时代一致：纯空白 TextNode 过滤、rich-text-block 折叠、absolute escapee
    /// 上浮收编）。
    /// 返回 (自身 tid, 冒泡中的 absolute escapee tids)。escapee = 「声明 absolute 且
    /// 任一 inset 显式」的子项，其 taffy 父不是 scene 父而是最近 positioned 祖先——
    /// 未遇到 positioned 祖先前随递归向上冒泡，positioned 节点收编（含根兜底）。
    fn sync(
        scene: &Scene,
        tree: &mut TaffyTree<MeasureContext>,
        ids: &mut Vec<Option<(NodeId, taffy::NodeId)>>,
        old: &[Option<(NodeId, taffy::NodeId)>],
        id: NodeId,
        parent_scroll: bool,
        image_sizes: &ImageSizeTable,
        root_size: (f32, f32),
        safe_insets: [f32; 4],
    ) -> (taffy::NodeId, Vec<taffy::NodeId>) {
        let node = scene.get_live(id, "layout/sync");
        let mut style = node.style.taffy_style.clone();
        // 视口相对长度（vw/vh/vmin/vmax/env()）按当帧 root_size/safe_insets 换算覆写
        // （分辨率适配的重排语言——root_size 随屏幕/适配模式变，声明 vw 的通道跟画布走）。
        if !node.style.viewport.is_empty() {
            node.style
                .viewport
                .apply(&mut style, root_size, safe_insets);
        }
        // 动画 layout override（#10）：覆写链末位（base → viewport → anim，动画最高
        // 优先级）。vw/vh/vmin/vmax 域按当帧 root_size 换算——动画中途 resize 自动
        // 重解析保持比例；px/pct 直落 taffy 原生形。set_style 值比较短路保证稳态帧
        // 零成本，动画帧逐帧值变 → 逐帧 set_style（taffy 内部标脏上溯，同 rematch 路径）。
        if let Some(a) = scene.anim.get(id) {
            if let Some(l) = a.width {
                style.size.width = anim_len_to_dimension(l, root_size);
            }
            if let Some(l) = a.height {
                style.size.height = anim_len_to_dimension(l, root_size);
            }
            if let Some(g) = a.flex_grow {
                style.flex_grow = g;
            }
        }
        // overflow != visible → 设 taffy overflow，让 flex automatic min-size=0（CSS flex §4.5）。
        // 不设则 taffy 默认 Visible → min-size=min-content → 容器被 content 撑开（viewport=content）
        // → overlap=0 → scroll 失效。
        style.overflow = taffy::geometry::Point {
            x: map_overflow(node.style.overflow_x),
            y: map_overflow(node.style.overflow_y),
        };
        // 滚动容器（Auto/Scroll）的直接子 flex-shrink=0：保持显式尺寸/min-content 溢出
        // （scroll 有效）。否则空内容子（如 .filler{height:300} min-content=0）被 shrink 到
        // viewport → overlap=0 → 不能滚。**只限滚动容器**——#64：overflow:hidden 是装饰性
        // 裁剪不是滚动语义，此前 hidden 也触发本规则 → .screen{hidden} 的弹性子
        // （min-height:0 链路）被锁死 shrink，viewport 被内容撑爆（浏览器预览却正常）。
        if parent_scroll {
            style.flex_shrink = 0.0;
        }
        let self_scroll = node.style.overflow_x == OverflowMode::Auto
            || node.style.overflow_x == OverflowMode::Scroll
            || node.style.overflow_y == OverflowMode::Auto
            || node.style.overflow_y == OverflowMode::Scroll;
        // CSS flex §4.5 automatic minimum size 的 specified-size suggestion 近似：
        // 显式 Length 尺寸的子项，浏览器以声明尺寸为 shrink 地板（76px 顶栏不会被
        // 溢出行按比例挤扁）；taffy 的 auto-min 走裸 min-content（空容器=0）→ 固定
        // 尺寸兄弟被挤扁、预览（真浏览器）不缩——#64 弹性视口修开后必现的连带偏差。
        // 只对 min 仍为 Auto 的通道生效（作者显式 min 声明永远赢）；percent/viewport
        // 形不碰（viewport 占位 Length(0) 地板=0 无害）。size 槽是 Dimension、min 槽是
        // LengthPercentageAuto（taffy 0.14 起分型），Length 地板经 expand 取值换型落位。
        for (size_slot, min_slot) in [
            (style.size.width, &mut style.min_size.width),
            (style.size.height, &mut style.min_size.height),
        ] {
            if let taffy::style::ExpandedDimension::Length(v) = size_slot.expand() {
                if min_slot.is_auto() {
                    *min_slot = LengthPercentageAuto::length(v);
                }
            }
        }
        // 叶子：Text/Image/文本控件装 MeasureContext。
        // TextField/TextArea/NumberField 是控件叶子（value/placeholder 存 ControlState，
        // 非 text_contents），须装 Text measure——否则 taffy content=0、高度只剩 padding，
        // 文字不参与布局（pivot 后空 div 形态暴露：高度塌成 padding-only）。
        //
        // rich-text-block 容器：编译 inline 子树成 RichRun，作 RichText 叶子测——inline
        // 子折进父的单段 inline flow（不递归进 taffy）。build 下方 children_ids 对
        // rich_text_block 返空 Vec 实现「不递归」。
        // display:flex 的策略切换（不折叠）在 rematch 应用 display 声明处翻转本 flag
        //（见 dynamic.rs rematch_pseudo_classes）——build 只认 flag，单一真相源。
        let ctx: Option<MeasureContext> = if node.rich_text_block {
            let s = &node.style;
            let runs = crate::text::rich_compile::compile_rich_runs(scene, id, image_sizes);
            Some(MeasureContext::RichText {
                runs,
                line_height: s.effective_line_height(),
                letter_spacing: s.letter_spacing,
                align: s.text_align,
                wrap: s.wrap_control(),
                family: s.font_family.clone(),
                h_inset: lp(s.taffy_style.padding.left)
                    + lp(s.taffy_style.padding.right)
                    + lp(s.taffy_style.border.left)
                    + lp(s.taffy_style.border.right),
            })
        } else {
            match &node.kind {
                NodeKind::TextNode => {
                    let s = &node.style;
                    Some(MeasureContext::Text {
                        content: scene.text_contents.get(&id).cloned().unwrap_or_default(),
                        font_size: s.font_size,
                        line_height: s.effective_line_height(),
                        letter_spacing: s.letter_spacing,
                        align: s.text_align,
                        wrap: s.wrap_control(),
                        family: s.font_family.clone(),
                        color: s.color,
                        font_weight: s.font_weight,
                        h_inset: lp(s.taffy_style.padding.left)
                            + lp(s.taffy_style.padding.right)
                            + lp(s.taffy_style.border.left)
                            + lp(s.taffy_style.border.right),
                    })
                }
                NodeKind::TextField | NodeKind::TextArea | NodeKind::NumberField => {
                    let s = &node.style;
                    // value 优先，空时用 placeholder（与 render 显示一致）；measure 用显示文本
                    // 算 intrinsic size，taffy 再加 padding/border → border-box 高度含文字行高。
                    // 追踪 is_placeholder：颜色用占位色（placeholder_render_color），与 render 一致
                    // ——颜色在此烘焙进缓存 TextLayout 的 per-run 色，render 复用缓存，故两处须同色。
                    let (content, is_placeholder) = scene
                        .controls
                        .get(id)
                        .and_then(|cs| match cs {
                            crate::scene::node::ControlState::TextField(e)
                            | crate::scene::node::ControlState::TextArea(e) => {
                                // 掩码与 measure_text_controls/render 同源（-webkit-text-security）。
                                let dv = crate::scene::control::display_value_masked(
                                    e,
                                    s.text_security.map(crate::scene::control::mask_char),
                                )
                                .0;
                                if dv.is_empty() {
                                    Some((e.placeholder.clone(), true))
                                } else {
                                    Some((dv, false))
                                }
                            }
                            crate::scene::node::ControlState::NumberField { edit, .. } => {
                                let dv = crate::scene::control::display_value_masked(
                                    edit,
                                    s.text_security.map(crate::scene::control::mask_char),
                                )
                                .0;
                                if dv.is_empty() {
                                    Some((edit.placeholder.clone(), true))
                                } else {
                                    Some((dv, false))
                                }
                            }
                            _ => None,
                        })
                        .unwrap_or_default();
                    Some(MeasureContext::Text {
                        content,
                        font_size: s.font_size,
                        line_height: s.effective_line_height(),
                        letter_spacing: s.letter_spacing,
                        align: s.text_align,
                        wrap: crate::style::resolved::control_wrap_control(s),
                        family: s.font_family.clone(),
                        color: if is_placeholder {
                            crate::style::resolved::placeholder_render_color(
                                s.placeholder_color,
                                s.color,
                            )
                        } else {
                            s.color
                        },
                        font_weight: s.font_weight,
                        h_inset: lp(s.taffy_style.padding.left)
                            + lp(s.taffy_style.padding.right)
                            + lp(s.taffy_style.border.left)
                            + lp(s.taffy_style.border.right),
                    })
                }
                NodeKind::Image => {
                    // Look up real intrinsic dims via the node's image src (side table).
                    // 借引用查 image_sizes——src 仅用于查表，无需每帧每图节点克隆 String。
                    let src = scene.image_srcs.get(&id).map(String::as_str).unwrap_or("");
                    let s = &node.style.taffy_style;
                    let (iw, ih) = image_sizes
                        .get(src)
                        .filter(|(w, h)| *w != 0 && *h != 0)
                        .map(|&(w, h)| (w as f32, h as f32))
                        .unwrap_or((64.0, 64.0));
                    Some(MeasureContext::Image {
                        iw,
                        ih,
                        w_dim: s.size.width,
                        h_dim: s.size.height,
                    })
                }
                _ => None,
            }
        };
        // min-width=0 让 flex-shrink 生效：taffy 默认 min-size:auto 会把 measure(None) 的
        // max-content 当 min-content，阻止 shrink → 长文本不收缩、超框。设 0 放开宽度。
        // 只设宽度：文本不纵向 shrink，min-height=0 无收益却有副作用——让 flex column 父
        // 容器主轴尺寸算大（按钮等容器被撑高、底图下沿往下拉），所以 height 保留 Auto。
        // 作者显式声明的 min-width 保留（如 stat-bar 的 label/val 固定列宽）——只在
        // 未声明（Auto）时才放开 shrink。
        // 复用路径同享（measure 叶子统一应用）：增量与全重建的期望态必须逐字节同源，
        // 漏一处即差分漂移。
        if ctx.is_some() && style.min_size.width.is_auto() {
            style.min_size.width = LengthPercentageAuto::length(0.0);
        }
        // 递归子节点（后序：子先同步，父随后挂期望子列表）。
        // 过滤纯空白 TextNode（HTML tag 间换行+缩进）——它们不应成 flex item 撑开父容器
        // 主轴或挤压兄弟（HTML 标准空白折叠行为）。被过滤的节点 taffy_ids[id.index()]
        // 保持 None，write_back 跳过、layout_rect 保持默认 0。
        //
        // rich-text-block：inline 子已被 compile_rich_runs 折进 RichText 叶子测，
        // **不递归进 taffy**——它们的 taffy_ids 保持 None，write_back 跳过、layout_rect
        // 保持默认 0（它们渲染进父 mesh，无独立 box；render 消费 text_layouts[父]）。
        // absolute 包含块（CSS 浏览器语义）：声明 absolute 且任一 inset 显式的子项，
        // taffy 父挂最近 positioned 祖先（position_declared != Static）而非 scene 父——
        // taffy 0.12 原生只按直接父布局 absolute，无「最近 positioned 祖先」概念，这里在
        // 建树期重挂补齐。inset 全 auto 的 absolute 保持直接父（浏览器 hypothetical-box
        // 静态位置语义不做，见 fence 文档已知限制）。
        let positioned =
            node.style.position_declared != crate::style::resolved::PositionDeclared::Static;
        let mut children_ids: Vec<taffy::NodeId> = Vec::new();
        let mut escaped: Vec<taffy::NodeId> = Vec::new();
        if !node.rich_text_block {
            for c in node.children.iter() {
                if is_whitespace_only_text(scene, *c) {
                    continue;
                }
                let (ctid, cesc) = sync(
                    scene,
                    tree,
                    ids,
                    old,
                    *c,
                    self_scroll,
                    image_sizes,
                    root_size,
                    safe_insets,
                );
                escaped.extend(cesc); // 下层冒上来的，随本层定位性收编或继续上浮
                let child = scene.get_live(*c, "layout/sync");
                let abs_escapee = child.style.taffy_style.position
                    == taffy::style::Position::Absolute
                    && child.style.position_declared
                        == crate::style::resolved::PositionDeclared::Absolute
                    && inset_any_explicit(&child.style.taffy_style.inset);
                if abs_escapee && !positioned {
                    escaped.push(ctid); // 本层非 positioned：继续向包含块候选上浮
                } else {
                    children_ids.push(ctid); // 含「absolute 子的包含块就是本层」的情形
                }
            }
        }
        if positioned {
            // 本层是包含块候选：收编全部下浮 escapee，一并挂名下（taffy 按 absolute 通道布局）。
            children_ids.append(&mut escaped);
        }

        // 复用或创建：老条目 NodeId 吻合同一节点 → 值比较短路 set_style /
        // set_node_context（taffy 内部标脏上溯）；否则（新节点 / slot 复用 gen 不符）
        // 创建新 taffy 节点。
        let tid = match old[id.index()].filter(|&(nid, _)| nid == id) {
            Some((_, tid)) => {
                if tree.style(tid).is_ok_and(|s| s != &style) {
                    tree.set_style(tid, style).ok();
                }
                if tree.get_node_context(tid) != ctx.as_ref() {
                    tree.set_node_context(tid, ctx).ok();
                }
                tid
            }
            None => match ctx {
                // 叶子：装测量上下文。children 应为空（Text/Image 是叶子）。
                Some(mctx) => tree.new_leaf_with_context(style, mctx).unwrap(),
                None => tree.new_with_children(style, &children_ids).unwrap(),
            },
        };
        ids[id.index()] = Some((id, tid));
        // 子列表同步：new_with_children 创建时已挂期望子（比较短路）；既有节点
        // 子序/成员漂移（增删/重排/重挂/escapee 迁移）才 set_children（内部自会从
        // 旧父摘挂 + 标脏上溯）。
        if tree.children(tid).is_ok_and(|cur| cur != children_ids) {
            tree.set_children(tid, &children_ids).ok();
        }
        (tid, escaped)
    }

    /// 任一 inset 边显式（非 auto）——absolute escapee 的判定条件之一。
    fn inset_any_explicit(
        inset: &taffy::geometry::Rect<taffy::style::LengthPercentageAuto>,
    ) -> bool {
        use taffy::style::LengthPercentageAuto;
        let auto = LengthPercentageAuto::AUTO;
        inset.left != auto || inset.right != auto || inset.top != auto || inset.bottom != auto
    }

    let (root_tid, escaped) = sync(
        scene,
        &mut cache.tree,
        &mut cache.ids,
        &old,
        scene.roots[0],
        false,
        image_sizes,
        root_size,
        safe_insets,
    );
    // 根收编余下 escapee：无任何 positioned 祖先时包含块 = 初始包含块（视口），CSS 语义。
    // 无条件 set_children（罕见路径：持续存在的 escapee 场景根每帧标脏，但子树缓存
    // 仍干净——taffy 脏传播只上溯，干净子树照跳）。
    if !escaped.is_empty() {
        let mut kids = cache.tree.children(root_tid).unwrap_or_default();
        kids.extend(escaped);
        cache.tree.set_children(root_tid, &kids).unwrap();
    }
    // 老条目清退：未被新映射保留的 taffy 节点统一 remove。remove 会把子节点的父引用
    // 清空（孤儿滞留树内）——被清退子树的每个节点都有自己的老条目，逐一 remove 兜净。
    for &(nid, tid) in old.iter().flatten() {
        let kept = matches!(cache.ids.get(nid.index()), Some(Some((_, t))) if *t == tid);
        if !kept {
            cache.tree.remove(tid).ok();
        }
    }

    // taffy NodeId → scene NodeId 反查，供 measure 闭包按 taffy nid 把 TextLayout
    // 存进 scene 索引的 text_layouts。render 复用，消除 layout/render 双测量不一致。
    let mut taffy_to_scene: HashMap<taffy::NodeId, NodeId> = HashMap::new();
    for &(nid, tid) in cache.ids.iter().flatten() {
        taffy_to_scene.insert(tid, nid);
    }
    // text_layouts 承接上帧（同 measure_cache 的 carry-over 模式）：增量 solve 的稳态帧
    // taffy 缓存全命中、measure 闭包不跑——若每帧新建全空，render 会退回用整数化
    // content_w 重测每个文本，短文本因 intrinsic 亚像素超宽误判换行（老病复发：
    // node.rs text_layouts 字段注释）。未重测节点的布局没变，上帧 TextLayout 仍准确；
    // 重测节点由闭包按 Some 优先规则覆写。新节点槽位由 alloc_node_slot 清 None，无串染。
    let mut text_layouts: Vec<Option<TextLayout>> = std::mem::take(&mut scene.text_layouts);
    // 平行写入代数表（同 carry-over 模式）：render 槽写入时 +1，A2 增量指纹消费。
    let mut text_layout_versions: Vec<u32> = std::mem::take(&mut scene.text_layout_versions);
    // measure memo：跨帧 carry-over。mem::take 出 scene（期间 scene.text_measure_cache 空），
    // 闭包用完在末尾写回——与 text_layouts 同模式，避 borrow 冲突（build 已在上方借过 scene）。
    let mut measure_cache: Vec<Option<crate::text::layout::TextMeasureCache>> =
        std::mem::take(&mut scene.text_measure_cache);
    let cap_need = scene.nodes.capacity() + 1;
    if text_layouts.len() < cap_need {
        text_layouts.resize(cap_need, None);
    }
    if text_layout_versions.len() < cap_need {
        text_layout_versions.resize(cap_need, 0);
    }
    if measure_cache.len() < cap_need {
        measure_cache.resize(cap_need, None);
    }

    // 设根 size：覆盖为调用方给的 root_size（viewport）。值比较短路——稳态帧
    //（viewport 未变）不 set_style，根保持干净缓存。
    // Style.size 字段类型是 Size<Dimension>（不是 LengthPercentageAuto）。
    //
    // 根的 box_sizing 在此覆写为 BorderBox：root_size 钉的是根的 **border box**（= 视口，
    // 浏览器 ICB 同构——Stage 拥有画布，作者声明不参与），padding/border 内缩 content 而非
    // 外溢。否则全局 ContentBox 钉（ResolvedStyle::default）把 root_size 解释成 content
    // box，root+padding 恒外溢出视口（#116：home root 1920+96=2016；root 声明被本处覆写，
    // 作者侧手算/包装层均救不回——「root+padding 内缩」是全屏页最高频惯用法，必须在喂入
    // 语义层修根因）。零 padding 根 BorderBox 与 ContentBox 等值，存量页无感。
    let root_style = cache.tree.style(root_tid).unwrap().clone();
    let sized_root = Style {
        size: Size {
            width: Dimension::length(root_size.0),
            height: Dimension::length(root_size.1),
        },
        box_sizing: taffy::style::BoxSizing::BorderBox,
        ..root_style.clone()
    };
    if sized_root != root_style {
        cache.tree.set_style(root_tid, sized_root).ok();
    }

    // solve：单一 FnMut 闭包按 context 分派。
    // known.width: Option<f32> —— Some=约束宽，None=不限（→ measure_text max_width=None）。
    // remeasured：本帧被重测过的节点（首测清 text_layouts 槽——恢复「帧内从空填充」
    // 语义，防 intrinsic-only 重测后残留上帧 constrained 行断；未重测节点保留
    // carry-over 值，见上方 text_layouts 注释）。
    let mut remeasured = vec![false; cap_need];
    cache
        .tree
        .compute_layout_with_measure(
            root_tid,
            Size::MAX_CONTENT,
            |input: LayoutInput,
             nid: taffy::NodeId,
             node_ctx: Option<&mut MeasureContext>,
             _style: &Style|
             -> LayoutOutput {
                let known = input.known_dimensions;
                match node_ctx {
                    // 无测量上下文的叶子（0.14 起闭包对一切无 children 节点调用，不只
                    // context 节点）：按 style 布局（同 taffy 内置默认）。0.12 期这里
                    // 返 ZERO 无害（闭包不会被调到）；0.14 空容器叶子实走此路，ZERO 会
                    // 把显式尺寸的空容器解成 0×0。
                    None => {
                        taffy::compute_leaf_layout(input, _style, |_, _| 0.0, |_, _| Size::ZERO)
                    }
                    Some(MeasureContext::Image {
                        iw,
                        ih,
                        w_dim,
                        h_dim,
                    }) => {
                        let (iw, ih, wd, hd) = (*iw, *ih, *w_dim, *h_dim);
                        // Dimension 是 compact tagged pointer，变体判定走 tag/value（0.14
                        // 另有 expand() 枚举口，这里 tag 判定足够）。
                        // width：known.width（Percent/fit 解析后，taffy 传）> css Length > 等比 height > intrinsic。
                        //   Percent width：taffy 第二次传 known.width=Some(解析宽)。
                        //
                        // 等比分支精确复刻升级前 match 臂 `(None, Dimension::Auto, Dimension::Length(h)) => h*iw/ih`：
                        // 仅 wd==Auto 时按 height 推宽。Percent width（无可解析父）落 intrinsic iw，
                        // 不混进 height-derive。
                        let wd_is_length = wd.tag() == taffy::style::CompactLength::LENGTH_TAG;
                        let hd_is_length = hd.tag() == taffy::style::CompactLength::LENGTH_TAG;
                        let w = if let Some(v) = known.width {
                            v
                        } else if wd_is_length {
                            wd.value()
                        } else if hd_is_length && wd.is_auto() {
                            hd.value() * iw / ih
                        } else {
                            iw
                        };
                        // height：css Length > known.height > 等比 width（CSS img height:auto 默认）。
                        let h = if hd_is_length {
                            hd.value()
                        } else if let Some(v) = known.height {
                            v
                        } else {
                            w * ih / iw
                        };
                        LayoutOutput::from_outer_size(Size {
                            width: w,
                            height: h,
                        })
                    }
                    Some(MeasureContext::Text {
                        content,
                        font_size,
                        line_height,
                        letter_spacing,
                        align,
                        wrap,
                        family,
                        color,
                        font_weight,
                        h_inset,
                    }) => {
                        // 0.14：measure 闭包即叶子布局算法（0.12 期 padding/min/max 合成
                        // 在 taffy 内部），委托 compute_leaf_layout 干 border-box 合成，
                        // 内层只测 content 尺寸。
                        let stack = fonts.stack_for(family.as_deref());
                        taffy::compute_leaf_layout(
                            input,
                            _style,
                            |_, _| 0.0,
                            |known_in: Size<Option<f32>>, avail_in: Size<AvailableSpace>| {
                                // 换行约束宽（content 域）：ComputeSize 轮 known_in 原样是
                                // border-box（扣 h_inset）；avail_in 的 Definite 已被
                                // compute_leaf_layout 扣过 content_box_inset（不重复扣）。
                                // known 缺席回退 avail 的 Definite 宽——定宽容器里 auto 宽
                                // 文本子（flex column 的 span 等），只用 known 会按
                                // max-content 量出单行超框（浏览器按可用宽换行）；
                                // MaxContent/MinContent 保持 None（走 intrinsic 测量）。
                                // ≤0（含 taffy sizing 轮的 Definite(0)/known=Some(0)）一律
                                // 视作无约束：0 宽盒内浏览器文本横向溢出而非逐字竖排，且
                                // 首个 Some(0) 测量会经 render 槽 Some-优先策略钉死成多行。
                                let mw = known_in
                                    .width
                                    .map(|w| (w - *h_inset).max(0.0))
                                    .or(match avail_in.width {
                                        AvailableSpace::Definite(w) => Some(w),
                                        _ => None,
                                    })
                                    .filter(|w| *w > f32::EPSILON);
                                let sid_opt = taffy_to_scene.get(&nid).copied();
                                if let Some(sid) = sid_opt {
                                    if !std::mem::replace(&mut remeasured[sid.index()], true) {
                                        text_layouts[sid.index()] = None;
                                    }
                                }
                                // measure memo：fingerprint 命中 → 复用 TextLayout 跳过 shaping。
                                // 两槽：mw=None→intrinsic（max-content），mw=Some→constrained（换行）。
                                // fingerprint 含 content hash → set_text / slot 换内容自动 miss；
                                // color / 字体解析链也进键（两者烙进 TextLayout，缺席 = 陈旧缓存）。
                                let font_ids: Vec<u32> = std::iter::once(stack.primary_id)
                                    .chain(stack.fallbacks.iter().map(|(_, id)| *id))
                                    .collect();
                                let fp = crate::text::layout::text_fingerprint(
                                    content,
                                    *font_size,
                                    *line_height,
                                    *letter_spacing,
                                    *align,
                                    *wrap,
                                    *font_weight,
                                    family.as_deref(),
                                    *color,
                                    &font_ids,
                                    mw,
                                );
                                let layout = if let Some(sid) = sid_opt {
                                    let entry = measure_cache[sid.index()].get_or_insert_with(
                                        crate::text::layout::TextMeasureCache::default,
                                    );
                                    let slot = if mw.is_none() {
                                        &mut entry.intrinsic
                                    } else {
                                        &mut entry.constrained
                                    };
                                    if slot.as_ref().is_some_and(|(f, _)| *f == fp) {
                                        slot.as_ref().unwrap().1.clone()
                                    } else {
                                        let l = measure_text(
                                            content,
                                            *font_size,
                                            *line_height,
                                            *letter_spacing,
                                            *align,
                                            *wrap,
                                            mw,
                                            &stack,
                                            *color,
                                            crate::text::rich::weight_from_font_weight(
                                                *font_weight,
                                            ),
                                        );
                                        *slot = Some((fp, l.clone()));
                                        l
                                    }
                                } else {
                                    // 无 scene 节点映射（文本过滤/边角）：不缓存。
                                    measure_text(
                                        content,
                                        *font_size,
                                        *line_height,
                                        *letter_spacing,
                                        *align,
                                        *wrap,
                                        mw,
                                        &stack,
                                        *color,
                                        crate::text::rich::weight_from_font_weight(*font_weight),
                                    )
                                };
                                // render 槽：存 TextLayout 供 render 复用。Some（available 测量）优先——
                                // 短文本 taffy 只传 None（max-content ≤ available，不换行），长文本传
                                // Some(available)（换行）。一旦存了 Some，后续 None 不覆盖。
                                if let Some(sid) = sid_opt {
                                    let rslot = &mut text_layouts[sid.index()];
                                    if rslot.is_none() || known_in.width.is_some() {
                                        *rslot = Some(layout.clone());
                                        text_layout_versions[sid.index()] += 1;
                                    }
                                }
                                Size {
                                    width: layout.text_width,
                                    height: layout.text_height,
                                }
                            },
                        )
                    }
                    Some(MeasureContext::RichText {
                        runs,
                        line_height,
                        letter_spacing,
                        align,
                        wrap,
                        family,
                        h_inset,
                        ..
                    }) => {
                        // RichText 走 measure_rich_text（简化 inline flow）。
                        // 回退走 FontStack（per-glyph 选字体）；run.font_id 仍是主字体 id。
                        // 委托结构同 Text（0.14 闭包即叶子算法，内层只测 content）。
                        let stack = fonts.stack_for(family.as_deref());
                        taffy::compute_leaf_layout(
                            input,
                            _style,
                            |_, _| 0.0,
                            |known_in: Size<Option<f32>>, avail_in: Size<AvailableSpace>| {
                                // 同 Text：known_in 是 border-box 扣 h_inset，avail_in 的
                                // Definite 已是 content 域（不重复扣）。
                                let mw = known_in
                                    .width
                                    .map(|w| (w - *h_inset).max(0.0))
                                    .or(match avail_in.width {
                                        AvailableSpace::Definite(w) => Some(w),
                                        _ => None,
                                    })
                                    .filter(|w| *w > f32::EPSILON);
                                let sid_opt = taffy_to_scene.get(&nid).copied();
                                if let Some(sid) = sid_opt {
                                    if !std::mem::replace(&mut remeasured[sid.index()], true) {
                                        text_layouts[sid.index()] = None;
                                    }
                                }
                                // 指纹 memo：runs 每帧现编译（便宜，O(inline 子)），
                                // 算指纹命中缓存跳过贵的 measure_rich_text（shaping）。span 换色/换内容
                                // → runs 变 → fp 变 → 自动 miss 重测（不依赖 dirty_text 传播）。
                                // 两槽 intrinsic/constrained（同 Text）：mw=None 走 intrinsic，
                                // mw=Some 走 constrained；约束宽量化进 fp 避亚像素抖动 thrash。
                                let fp = crate::text::layout::rich_text_fingerprint(
                                    runs,
                                    *line_height,
                                    *letter_spacing,
                                    *align,
                                    *wrap,
                                    family.as_deref(),
                                    mw,
                                );
                                let layout = if let Some(sid) = sid_opt {
                                    let entry = measure_cache[sid.index()].get_or_insert_with(
                                        crate::text::layout::TextMeasureCache::default,
                                    );
                                    let slot = if mw.is_none() {
                                        &mut entry.intrinsic
                                    } else {
                                        &mut entry.constrained
                                    };
                                    if slot.as_ref().is_some_and(|(f, _)| *f == fp) {
                                        slot.as_ref().unwrap().1.clone()
                                    } else {
                                        let l = crate::text::layout::measure_rich_text(
                                            runs,
                                            mw,
                                            *line_height,
                                            *letter_spacing,
                                            *align,
                                            *wrap,
                                            &stack,
                                        );
                                        *slot = Some((fp, l.clone()));
                                        l
                                    }
                                } else {
                                    // 无 scene 节点映射（边角）：不缓存，直接测。
                                    crate::text::layout::measure_rich_text(
                                        runs,
                                        mw,
                                        *line_height,
                                        *letter_spacing,
                                        *align,
                                        *wrap,
                                        &stack,
                                    )
                                };
                                // render 槽：存 TextLayout 供 render 复用（同 Text 的 Some 优先策略：
                                // 已存 Some 且本次 None 不覆盖；本次 Some 则覆盖）。
                                if let Some(sid) = sid_opt {
                                    let slot = &mut text_layouts[sid.index()];
                                    if slot.is_none() || known_in.width.is_some() {
                                        *slot = Some(layout.clone());
                                        text_layout_versions[sid.index()] += 1;
                                    }
                                }
                                Size {
                                    width: layout.text_width,
                                    height: layout.text_height,
                                }
                            },
                        )
                    }
                }
            },
        )
        .ok();

    // taffy 树绝对坐标预计算：absolute escapee 的 taffy 父 ≠ scene 父，layout.location
    // 相对的是 taffy 父——scene 递归累加父 origin 的旧算法对重挂节点会错位。沿 taffy
    // 树一次性累加得每节点绝对坐标（非重挂时与 scene 树累加同值，两树同构）。
    let mut taffy_abs: HashMap<taffy::NodeId, (f32, f32)> = HashMap::new();
    fn walk_taffy_abs(
        tree: &TaffyTree<MeasureContext>,
        tid: taffy::NodeId,
        origin: (f32, f32),
        out: &mut HashMap<taffy::NodeId, (f32, f32)>,
    ) {
        let Ok(layout) = tree.layout(tid) else {
            return;
        };
        let abs = (origin.0 + layout.location.x, origin.1 + layout.location.y);
        out.insert(tid, abs);
        if let Ok(kids) = tree.children(tid) {
            for c in kids {
                walk_taffy_abs(tree, c, abs, out);
            }
        }
    }
    walk_taffy_abs(&cache.tree, root_tid, (0.0, 0.0), &mut taffy_abs);

    // 回写 layout_rect + clip_rect（绝对坐标取 taffy 树累加值）。
    fn write_back(
        scene: &mut Scene,
        tree: &TaffyTree<MeasureContext>,
        ids: &[Option<(NodeId, taffy::NodeId)>],
        taffy_abs: &HashMap<taffy::NodeId, (f32, f32)>,
        id: NodeId,
    ) {
        // 被过滤的节点（纯空白 TextNode / rich 折叠子）：ids 槽为 None，layout_rect 保持
        // 默认 0。早返，跳过 solve 结果回写——但递归子节点（无，TextNode 是叶子），安全。
        let tid = match ids[id.index()] {
            Some((_, tid)) => tid,
            None => return,
        };
        let layout = tree.layout(tid).unwrap();
        let (x, y) = taffy_abs
            .get(&tid)
            .copied()
            .unwrap_or((layout.location.x, layout.location.y));
        let (w, h) = (layout.size.width, layout.size.height);
        let node = scene.get_live_mut(id, "layout/write_back");
        node.layout_rect = Rect { x, y, w, h };
        // clip_rect 按 rematch 后的 style.overflow 重派生（而非仅填充 create 时建的 Some 槽）。
        // 原因：<style> class 规则设的 overflow 走 dynamic_rules 运行时应用，打包期
        // base_style 无 overflow → create_node_from_template 时 clip_rect=None。若这里
        // 只填已有的 Some，rematch 后 overflow 虽设上但 clip_rect 仍 None → render 不裁剪。
        // 现在：任一轴 非 Visible → clip = 自身 border 框（解出的 layout_rect）；否则 None。
        let should_clip = node.style.overflow_x != OverflowMode::Visible
            || node.style.overflow_y != OverflowMode::Visible;
        node.clip_rect = if should_clip {
            Some(Rect { x, y, w, h })
        } else {
            None
        };
        let kids = node.children.clone();
        for c in kids {
            write_back(scene, tree, ids, taffy_abs, c);
        }
    }
    write_back(scene, &cache.tree, &cache.ids, &taffy_abs, scene.roots[0]);
    // layout 阶段 TextLayout 缓存交还 scene，供 render 复用（不重测）。
    scene.text_layouts = text_layouts;
    scene.text_layout_versions = text_layout_versions;
    // measure memo 写回（跨帧持久）。
    scene.text_measure_cache = measure_cache;
    // 持久 taffy 树写回（增量 solve 载体）。
    scene.layout_cache = cache;
}

/// 全重建 solve：清空持久 taffy 树后走同一增量路径（从零 sync = 全量创建）。
/// 差分守卫测试的基准路径 + 诊断兜底（怀疑增量态腐坏时对照）。
pub fn solve_rebuild(
    scene: &mut Scene,
    fonts: &FontTable,
    root_size: (f32, f32),
    safe_insets: [f32; 4],
    image_sizes: &ImageSizeTable,
) {
    scene.layout_cache = LayoutCache::default();
    solve(scene, fonts, root_size, safe_insets, image_sizes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{NodeKind, Scene};
    use crate::style::resolved::ResolvedStyle;

    fn font_table() -> Option<FontTable> {
        let path = format!(
            "{}/tests/fixtures/DejaVuSans.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes = std::fs::read(&path).ok()?;
        let mut ft = FontTable::new();
        ft.register("DejaVu", bytes, true).ok()?;
        Some(ft)
    }

    /// 测试辅助：空图尺寸表（无 path → 全 64×64 兜底）。
    fn empty_sizes() -> ImageSizeTable {
        HashMap::new()
    }

    /// 测试辅助：建单条 path→(w,h) 尺寸表。
    fn sizes(path: &str, w: u32, h: u32) -> ImageSizeTable {
        let mut m = HashMap::new();
        m.insert(path.to_string(), (w, h));
        m
    }

    /// Image measure 三档优先级（CSS Length > 真实像素 > 64×64 兜底）。
    /// 用 Scene::build 手搓 Image scene。
    ///
    /// **布局陷阱**：`solve` 会用 `root_size` 覆盖根节点的 taffy size（见 prod
    /// `set_style(... size: Length(root_size) ...)`），故 Image 不能做根——否则
    /// 其 MeasureContext 的 intrinsic 尺寸被 root_size 强制覆盖，测不出三档。
    /// 包一层 Container 根（idx 0），Image 做 leaf 子（idx 1），其 measure 值才生效。
    #[test]
    fn image_css_length_overrides_intrinsic() {
        // CSS width:100px height:50px → CSS 声明赢（覆盖 intrinsic 真实像素 / 64×64 兜底）。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.size.width = Dimension::length(100.0);
        img_style.taffy_style.size.height = Dimension::length(50.0);
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Image,
                img_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                Some("x.png".into()),
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (300.0, 300.0),
            [0.0; 4],
            &sizes("x.png", 40, 20),
        );
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect; // Image 是 root 唯一子
        assert!(
            (r.w - 100.0).abs() < 0.1,
            "CSS length 赢：w=100，got {}",
            r.w
        );
        assert!((r.h - 50.0).abs() < 0.1, "CSS length 赢：h=50，got {}", r.h);
    }

    /// 辅助：手搓 .screen > .wrap(relative, margin-left) > .card > btn(absolute) 形态。
    /// 返回 (scene, btn NodeId)。wrap_off = wrap 相对根的 x 偏移（margin 实现）。
    fn abs_containing_block_scene(
        wrap_relative: bool,
        mid_relative: bool,
        btn_inset: bool,
    ) -> (Scene, crate::scene::NodeId) {
        use crate::style::resolved::PositionDeclared;
        use taffy::style::LengthPercentageAuto;

        let mut wrap_style = ResolvedStyle::default();
        wrap_style.taffy_style.size.width = Dimension::length(500.0);
        wrap_style.taffy_style.size.height = Dimension::length(400.0);
        wrap_style.taffy_style.margin.left = LengthPercentageAuto::length(100.0);
        if wrap_relative {
            wrap_style.position_declared = PositionDeclared::Relative;
        }

        let mut mid_style = ResolvedStyle::default(); // card：无定位
        mid_style.taffy_style.size.width = Dimension::length(200.0);
        mid_style.taffy_style.size.height = Dimension::length(100.0);
        if mid_relative {
            mid_style.taffy_style.margin.top = LengthPercentageAuto::length(30.0);
            mid_style.position_declared = PositionDeclared::Relative;
        }

        let mut btn_style = ResolvedStyle::default();
        btn_style.taffy_style.position = taffy::style::Position::Absolute;
        btn_style.position_declared = PositionDeclared::Absolute;
        btn_style.taffy_style.size.width = Dimension::length(20.0);
        btn_style.taffy_style.size.height = Dimension::length(10.0);
        if btn_inset {
            btn_style.taffy_style.inset.top = LengthPercentageAuto::length(40.0);
            btn_style.taffy_style.inset.right = LengthPercentageAuto::length(56.0);
        }

        let e = |p, kind, st| (p, kind, st, Vec::new(), None, false, None, None, None, None);
        let entries = [
            e(None, NodeKind::Container, ResolvedStyle::default()),
            e(Some(0), NodeKind::Container, wrap_style),
            e(Some(1), NodeKind::Container, mid_style),
            e(Some(2), NodeKind::Container, btn_style),
        ];
        let scene = Scene::build(&entries);
        let root = scene.roots[0];
        let wrap = scene.get(root).unwrap().children[0];
        let card = scene.get(wrap).unwrap().children[0];
        let btn = scene.get(card).unwrap().children[0];
        (scene, btn)
    }

    /// btn 的包含块 = 最近 positioned 祖先 .wrap（非直接父 .card）。
    /// 浏览器：x = wrap.x + wrap.w - right - btn.w；旧实现（直接父）= card.x + card.w - ...
    #[test]
    fn absolute_resolves_against_nearest_positioned_ancestor() {
        let (mut scene, btn) = abs_containing_block_scene(true, false, true);
        let fonts = FontTable::new();
        solve(
            &mut scene,
            &fonts,
            (1920.0, 1080.0),
            [0.0; 4],
            &empty_sizes(),
        );
        let r = scene.get(btn).unwrap().layout_rect;
        let expect_x = 100.0 + 500.0 - 56.0 - 20.0; // wrap 右内缘 - right - 宽
        assert!(
            (r.x - expect_x).abs() < 0.5,
            "包含块 = wrap：x≈{expect_x}（直接父 card 会得 ≈224），got {}",
            r.x
        );
        assert!(
            (r.y - 40.0).abs() < 0.5,
            "top 相对 wrap 顶部（wrap 无上偏移），got {}",
            r.y
        );
    }

    /// 中间还有一个 positioned 节点时，最近者胜（mid.margin-top=30 参与坐标）。
    #[test]
    fn absolute_nearest_positioned_wins_over_outer_one() {
        let (mut scene, btn) = abs_containing_block_scene(true, true, true);
        let fonts = FontTable::new();
        solve(
            &mut scene,
            &fonts,
            (1920.0, 1080.0),
            [0.0; 4],
            &empty_sizes(),
        );
        let r = scene.get(btn).unwrap().layout_rect;
        // mid 含上边距 30：top 相对 mid（border box 顶）= 30 + 40 = 70。
        assert!(
            (r.y - 70.0).abs() < 0.5,
            "最近 positioned（mid, margin-top 30）赢：y≈70，got {}",
            r.y
        );
    }

    /// 无任何 positioned 祖先 → 初始包含块（视口）：相对根而非中间层。
    #[test]
    fn absolute_without_positioned_ancestor_uses_viewport() {
        let (mut scene, btn) = abs_containing_block_scene(false, false, true);
        // 场景只设了 top/right：top 相对视口 = 40（wrap/card 偏移不参与）。
        let fonts = FontTable::new();
        solve(
            &mut scene,
            &fonts,
            (1920.0, 1080.0),
            [0.0; 4],
            &empty_sizes(),
        );
        let r = scene.get(btn).unwrap().layout_rect;
        assert!(
            (r.y - 40.0).abs() < 0.5,
            "无 positioned 祖先 → 视口：y≈40，got {}",
            r.y
        );
        let expect_x = 1920.0 - 56.0 - 20.0; // right 相对视口右缘
        assert!(
            (r.x - expect_x).abs() < 0.5,
            "right 相对视口：x≈{expect_x}，got {}",
            r.x
        );
    }

    /// inset 全 auto 的 absolute 不重挂（保持直接父的静态位置语义，fence 已知限制）。
    #[test]
    fn absolute_without_inset_stays_with_direct_parent() {
        let (mut scene, btn) = abs_containing_block_scene(true, false, false);
        let fonts = FontTable::new();
        solve(
            &mut scene,
            &fonts,
            (1920.0, 1080.0),
            [0.0; 4],
            &empty_sizes(),
        );
        let r = scene.get(btn).unwrap().layout_rect;
        let root = scene.roots[0];
        let wrap = scene.get(root).unwrap().children[0];
        let card = scene.get(wrap).unwrap().children[0];
        let card_r = scene.get(card).unwrap().layout_rect;
        assert!(
            (r.x - card_r.x).abs() < 0.5 && (r.y - card_r.y).abs() < 0.5,
            "无 inset absolute 静态位置在直接父 card 内容区起点 ({}, {})，got ({}, {})",
            card_r.x,
            card_r.y,
            r.x,
            r.y
        );
    }

    /// 无 CSS 尺寸 → 用尺寸表真实像素（40×20）。
    #[test]
    fn image_measure_uses_real_dims_when_no_css() {
        // 无 CSS 尺寸 + 尺寸表有 x.png=40×20 → intrinsic = 40×20（真实像素）。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.align_self = Some(AlignSelf::FLEX_START);
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Image,
                img_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                Some("x.png".into()),
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (300.0, 300.0),
            [0.0; 4],
            &sizes("x.png", 40, 20),
        );
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect; // Image 是 root 唯一子
        assert!((r.w - 40.0).abs() < 0.1, "真实像素：w=40，got {}", r.w);
        assert!((r.h - 20.0).abs() < 0.1, "真实像素：h=20，got {}", r.h);
    }

    /// 无 CSS + 尺寸表无 path / w,h=0 → 64×64 兜底（三档第三档）。
    #[test]
    fn image_measure_uses_64_fallback_when_no_size_entry() {
        // 无 CSS + 尺寸表无 x.png → 64×64 兜底。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.align_self = Some(AlignSelf::FLEX_START);
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Image,
                img_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                Some("x.png".into()),
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(&mut scene, &fonts, (300.0, 300.0), [0.0; 4], &empty_sizes());
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        assert!((r.w - 64.0).abs() < 0.1, "兜底：w=64，got {}", r.w);
        assert!((r.h - 64.0).abs() < 0.1, "兜底：h=64，got {}", r.h);
    }

    /// 尺寸表 w/h=0（非 PNG / 读失败）→ fallback 64×64。
    #[test]
    fn image_measure_falls_back_to_64_when_zero_dims() {
        // 尺寸表 x.png=(0,0)（非 PNG 兜底）→ fallback 64×64。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.align_self = Some(AlignSelf::FLEX_START);
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Image,
                img_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                Some("x.png".into()),
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (300.0, 300.0),
            [0.0; 4],
            &sizes("x.png", 0, 0),
        );
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        assert!((r.w - 64.0).abs() < 0.1, "w/h=0 → 兜底 w=64，got {}", r.w);
        assert!((r.h - 64.0).abs() < 0.1, "w/h=0 → 兜底 h=64，got {}", r.h);
    }

    /// img style="width:80px" + 真实 40×20 → height 等比 = 40（80×20/40，2:1 aspect）。
    #[test]
    fn image_measure_scales_height_to_width_aspect() {
        // img style="width:80px" intrinsic 40×20（真实，2:1）→ height 等比 = 40（80×20/40）。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.size.width = Dimension::length(80.0);
        img_style.taffy_style.align_self = Some(AlignSelf::FLEX_START);
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Image,
                img_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                Some("x.png".into()),
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (300.0, 300.0),
            [0.0; 4],
            &sizes("x.png", 40, 20),
        );
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        assert!((r.w - 80.0).abs() < 0.1, "w=80 (CSS)");
        assert!(
            (r.h - 40.0).abs() < 0.1,
            "h 等比=40（80×20/40，2:1 真实 aspect），got {}",
            r.h
        );
    }

    /// img style="height:60px" + 真实 40×20 → width 等比 = 120（60×40/20，2:1 aspect）。
    #[test]
    fn image_measure_scales_width_to_height_aspect() {
        // 只设 height：style="height:60px" intrinsic 40×20（真实，2:1）→ width 等比 = 120（60×40/20）。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.size.height = Dimension::length(60.0);
        img_style.taffy_style.align_self = Some(AlignSelf::FLEX_START);
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Image,
                img_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                Some("x.png".into()),
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (300.0, 300.0),
            [0.0; 4],
            &sizes("x.png", 40, 20),
        );
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        assert!((r.h - 60.0).abs() < 0.1, "h=60 (CSS)");
        assert!(
            (r.w - 120.0).abs() < 0.1,
            "w 等比=120（60×40/20，2:1 真实 aspect），got {}",
            r.w
        );
    }

    /// 纯空白 TextNode（HTML 元素间的换行+缩进）不应成 flex item 撑开父容器。
    ///
    /// HTML 标准行为：block/flex 容器子节点间的纯空白应折叠，不成 box/item。
    /// 修前根因：layout::build 把空白 TextNode 当 flex item，每个占一行行高
    /// （line-height 撑高）→ 后续兄弟节点被推下去 + flex-shrink:1 把它当
    /// shrinkable 内容压缩 → 卡片 img 被压成 19×48（应 48×48）。
    /// 修后：空白 TextNode 不进 taffy 树，layout_rect 保持默认 0。
    #[test]
    fn whitespace_only_text_does_not_open_flex_item() {
        // 建模：flex column 容器 > [空白 TextNode, Button]。
        // 期望：Button.y == 0（空白 text 不撑开父容器主轴）。
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::TextNode,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                Some("\n    ".into()),
                None,
            ),
            (
                Some(0),
                NodeKind::Button,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(&mut scene, &fonts, (300.0, 300.0), [0.0; 4], &empty_sizes());
        let children = &scene.get(scene.roots[0]).unwrap().children;
        // TextNode 在 children[0]，Button 在 children[1]。
        let ws_id = children[0];
        let btn_id = children[1];
        let ws = scene.get(ws_id).unwrap();
        let btn = scene.get(btn_id).unwrap();
        // 空白 text 不应占主轴空间——layout_rect.h 应保持默认 0。
        assert!(
            ws.layout_rect.h.abs() < 0.1,
            "空白 TextNode h 应 0（不撑开），got {}",
            ws.layout_rect.h
        );
        // Button 应顶在 y=0（不被空白 text 推下去）。
        assert!(
            btn.layout_rect.y.abs() < 0.1,
            "Button y 应 0（空白 text 不撑开），got {}",
            btn.layout_rect.y
        );
    }

    /// 含非空白字符的 TextNode 不被过滤（防误伤 inline 间的有意空格）。
    #[test]
    fn non_whitespace_text_keeps_layout_space() {
        // "Buy" 含字母 → 正常占 flex item 空间。
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::TextNode,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                Some("Buy".into()),
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(&mut scene, &fonts, (300.0, 300.0), [0.0; 4], &empty_sizes());
        let text_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = scene.get(text_id).unwrap().layout_rect;
        assert!(
            r.w > 1.0 && r.h > 1.0,
            "非空白 text 应正常测出尺寸，got w={} h={}",
            r.w,
            r.h
        );
    }

    /// rich-text-block 容器在 solve 期折叠 inline 子为单段 inline flow：build() 编译 runs
    /// → `MeasureContext::RichText` 叶子（子不递归进 taffy）→ measure 闭包走 RichText arm
    /// 调 `measure_rich_text` → TextLayout 存 `scene.text_layouts[div]`。
    ///
    /// 验收：长 ASCII 文本在窄宽（100px）下换行 → text_height / layout_rect.h 反映多行
    /// （远大于单行行高）；inline 子（TextNode）保持默认 layout_rect（无独立 box）。
    /// solve 折叠的核心契约。
    #[test]
    fn rich_text_block_measures_as_leaf_with_wrapping() {
        // root(structural Container) > div(rich_text_block, explicit width 100) > TextNode
        // 长文本。div 显式宽 100 → taffy 以 known.width=Some(100) 测 → measure_rich_text
        // 换行 → 多行。作 root 固定尺寸叶子测不到约束宽（taffy 不重测固定尺寸），故
        // 必须作子+显式宽驱动。
        let mut div_style = ResolvedStyle::default();
        div_style.taffy_style.size.width = Dimension::length(100.0);
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Container,
                div_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(1),
                NodeKind::TextNode,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                Some("The quick brown fox jumps over the lazy dog".into()),
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        let div = scene.get(scene.roots[0]).unwrap().children[0];
        scene.get_mut(div).unwrap().rich_text_block = true;
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (300.0, 1000.0),
            [0.0; 4],
            &empty_sizes(),
        );
        let layout = scene.text_layouts[div.index()]
            .as_ref()
            .expect("rich-text-block solve 应填 text_layouts[div]");
        // 单行行高（font 16 × NORMAL_LINE_HEIGHT 1.31 ≈ 21）。多行 text_height 远大于此。
        let single_line_h = 16.0 * 1.31;
        assert!(
            layout.text_height > single_line_h * 2.0,
            "rich text 应换行多行，text_height={:.1} 应 > 2×单行({:.1})",
            layout.text_height,
            single_line_h * 2.0
        );
        // layout_rect.h（taffy 解出的 border-box 高）= measure 返的 height，同样反映多行。
        let r = &scene.get(div).unwrap().layout_rect;
        assert!(
            r.h > single_line_h * 2.0,
            "div layout_rect.h={:.1} 应 > 2×单行({:.1})，反映多行换行",
            r.h,
            single_line_h * 2.0
        );
        // 折叠的 inline 子（TextNode）保持默认 layout_rect（不进 taffy，无独立 box；
        // write_back 跳过 taffy_ids=None 的节点）。
        let tn = scene.get(div).unwrap().children[0];
        let tn_rect = scene.get(tn).unwrap().layout_rect;
        assert!(
            tn_rect.w.abs() < 0.1 && tn_rect.h.abs() < 0.1,
            "folded inline child 应无独立 layout_rect（保持默认 0），got {:?}",
            tn_rect
        );
    }

    /// 回归守卫：rich_text_block=false 的 Container 仍走 `new_with_children`，
    /// 子 TextNode 正常进 taffy 测 + 走 Text measure arm（不被 rich 分支误伤）。
    /// 与上一个 rich 测试互为正反：rich 折叠 / 非 rich 正常递归。
    #[test]
    fn non_rich_text_container_recurses_children_into_taffy() {
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::TextNode,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                Some("Buy".into()),
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        // rich_text_block 保持默认 false → 子 TextNode 走原 Text measure（独立 box）。
        let fonts = font_table().expect("need font");
        solve(&mut scene, &fonts, (300.0, 300.0), [0.0; 4], &empty_sizes());
        let text_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = scene.get(text_id).unwrap().layout_rect;
        assert!(
            r.w > 1.0 && r.h > 1.0,
            "structural container 子 TextNode 应正常测出尺寸，got w={} h={}",
            r.w,
            r.h
        );
        // TextNode 走 Text measure arm → 有独立 text_layouts 条目（非父 div 的 RichText 槽）。
        assert!(
            scene.text_layouts[text_id.index()].is_some(),
            "structural TextNode 应有独立 text_layouts 条目（走 Text arm，非 fold）"
        );
    }

    /// 回归（showcase quick-bar 裁剪丢失）：overflow 由 <style> class 规则设（运行时
    /// rematch 应用），打包期 base_style 无 overflow → create_node_from_template 时
    /// clip_rect=None。rematch 后 style.overflow 被设上，但 clip_rect 若不重派生 →
    /// render 不开 clip mask → 内容溢出可见。solve 的 write_back 必须按 rematch 后的
    /// style.overflow 重派生 clip_rect（而非仅填充已有 Some 槽）。
    #[test]
    fn clip_rect_rederived_from_rematched_overflow() {
        // 建 root：base_style overflow 双轴 Visible（clip_rect=None，模拟 class 规则未烘进 base）。
        let mut root_style = ResolvedStyle::default();
        root_style.overflow_x = OverflowMode::Visible;
        root_style.overflow_y = OverflowMode::Visible;
        let entries = [
            (
                None,
                NodeKind::Container,
                root_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        // 模拟 rematch 把 overflow-y 设上（class 规则 .quick-bar{overflow-x:auto} 在运行时应用）。
        let root = scene.roots[0];
        scene.get_mut(root).unwrap().style.overflow_x = OverflowMode::Auto;
        scene.get_mut(root).unwrap().style.overflow_y = OverflowMode::Visible;
        // create 时 base 无 overflow → clip_rect 是 None（重现在 bug 现场）。
        assert!(
            scene.get(root).unwrap().clip_rect.is_none(),
            "建节点时 base 无 overflow → clip None"
        );
        let fonts = font_table().expect("need font");
        solve(&mut scene, &fonts, (300.0, 300.0), [0.0; 4], &empty_sizes());
        // solve 后：style.overflow_x=Auto（rematched）→ clip_rect 应被重派生为 Some(解出的 rect)。
        let clip = scene.get(root).unwrap().clip_rect;
        assert!(
            clip.is_some(),
            "rematch 后 overflow 非 Visible → solve 应重派生 clip_rect"
        );
        let r = clip.unwrap();
        assert!(
            (r.w - 300.0).abs() < 1e-2 && (r.h - 300.0).abs() < 1e-2,
            "clip_rect 应=root border box (300,300)，got {:?}",
            r
        );
    }

    /// flex column + align-items:center + 定宽容器内的无宽
    /// rich-text-block 文本必须单行横排（浏览器一致先验）。
    ///
    /// 回归动机：测宽链路曾把可用宽度解析成 0 → `measure_text` 以 max_w=0 逐字换行
    /// （运行时竖排、浏览器预览横排）。缓解写法 `width:100%` 之所以有效，正是因为它
    /// 给了确定的 known width 绕开了该链路。
    #[test]
    fn flex_column_centered_auto_width_text_stays_single_line() {
        // root(structural) > .qi-pool(flex column, align-items:center, width:190)
        //   > .qi-label(rich_text_block, 无显式宽) > TextNode "气 3 / 4"
        let mut pool_style = ResolvedStyle::default();
        pool_style.taffy_style.display = taffy::Display::Flex;
        pool_style.taffy_style.flex_direction = taffy::FlexDirection::Column;
        pool_style.taffy_style.align_items = Some(taffy::AlignItems::CENTER);
        pool_style.taffy_style.size.width = Dimension::length(190.0);
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Container,
                pool_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(1),
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                Some("qi 3 / 4".into()),
                None,
            ),
            (
                Some(2),
                NodeKind::TextNode,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                Some("qi 3 / 4".into()),
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        let pool = scene.get(scene.roots[0]).unwrap().children[0];
        let label = scene.get(pool).unwrap().children[0];
        scene.get_mut(label).unwrap().rich_text_block = true;
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (300.0, 1000.0),
            [0.0; 4],
            &empty_sizes(),
        );
        let layout = scene.text_layouts[label.index()]
            .as_ref()
            .expect("rich-text-block solve 应填 text_layouts[label]");
        let single_line_h = 16.0 * 1.31;
        assert!(
            layout.text_height <= single_line_h * 1.5,
            "无宽文本在 flex column 居中容器下应单行横排，text_height={:.1} \
             （逐字竖排 ≈ {} 行）",
            layout.text_height,
            (layout.text_height / single_line_h).round()
        );
    }

    /// 视口相对长度端到端：width:50vw 在 root (800,600) solve → 400px；root_size
    /// 变（分辨率适配 set_root_size / resize）→ 下次 solve 跟随。分辨率适配的重排语言。
    #[test]
    fn viewport_width_resolves_against_root_size() {
        use crate::style::mapping::apply_decl;
        let mut st = ResolvedStyle::default();
        assert!(apply_decl(&mut st, "width", "50vw"));
        assert!(apply_decl(&mut st, "height", "10vh"));
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Container,
                st,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (800.0, 600.0),
            [0.0; 4],
            &HashMap::new(),
        );
        let id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(id).unwrap().layout_rect;
        assert!((r.w - 400.0).abs() < 0.1, "50vw @800 -> 400, got {}", r.w);
        assert!((r.h - 60.0).abs() < 0.1, "10vh @600 -> 60, got {}", r.h);
        // resize 后重排跟随
        solve(
            &mut scene,
            &fonts,
            (1000.0, 500.0),
            [0.0; 4],
            &HashMap::new(),
        );
        let r = &scene.get(id).unwrap().layout_rect;
        assert!((r.w - 500.0).abs() < 0.1, "50vw @1000 -> 500, got {}", r.w);
        assert!((r.h - 50.0).abs() < 0.1, "10vh @500 -> 50, got {}", r.h);
    }

    /// #109 地基回归：MeasureContext::Text 含 color，但 text_fingerprint 曾缺席 →
    /// 只改 style.color 时二次 solve 命中 measure 缓存，text_layouts 的 run.color
    /// （纯文本上屏唯一通道，烙进 mesh 顶点色）保持旧色。指纹补 color 后必 miss 重测。
    #[test]
    fn text_layout_refreshes_on_color_only_change() {
        let mut red = ResolvedStyle::default();
        red.color = [1.0, 0.2, 0.2, 1.0];
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::TextNode,
                red,
                Vec::new(),
                None,
                false,
                None,
                None,
                Some("hi".into()),
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (800.0, 600.0),
            [0.0; 4],
            &HashMap::new(),
        );
        let tid = scene.get(scene.roots[0]).unwrap().children[0];
        let run_color = |scene: &Scene| {
            scene.text_layouts[tid.index()]
                .as_ref()
                .expect("text node measured")
                .lines[0]
                .runs[0]
                .color
        };
        assert_eq!(run_color(&scene), [1.0, 0.2, 0.2, 1.0]);
        scene.get_mut(tid).unwrap().style.color = [0.2, 1.0, 0.2, 1.0];
        solve(
            &mut scene,
            &fonts,
            (800.0, 600.0),
            [0.0; 4],
            &HashMap::new(),
        );
        assert_eq!(
            run_color(&scene),
            [0.2, 1.0, 0.2, 1.0],
            "只改 color 也必须重测（指纹含 color 前：命中缓存 → 旧色陈旧）"
        );
    }

    /// #64 取证：Tripawd 地图链 .screen{overflow:hidden, flex column} >
    /// .map-area{flex-grow:1, min-height:0} > .map-scroll{flex-grow:1, min-height:0,
    /// overflow:auto} > .map-layer{显式 1572px}。期望 map-area/map-scroll 钳在 flex
    /// 份额（1004），layer 溢出可滚；修前被内容撑到 1572 → viewport==content → 不能滚。
    #[test]
    fn flex_min_height_zero_scroll_viewport_64_repro() {
        let mut screen = ResolvedStyle::default(); // .screen：flex column + overflow:hidden
        screen.taffy_style.flex_direction = taffy::style::FlexDirection::Column;
        screen.taffy_style.size.width = Dimension::length(1920.0);
        screen.taffy_style.size.height = Dimension::length(1080.0);
        screen.overflow_y = crate::style::resolved::OverflowMode::Hidden;

        let mut topbar = ResolvedStyle::default(); // 76px 顶栏（占位）
        topbar.taffy_style.size.width = Dimension::length(1920.0);
        topbar.taffy_style.size.height = Dimension::length(76.0);

        let mut area = ResolvedStyle::default(); // .map-area：grow + min-height:0
        area.taffy_style.flex_grow = 1.0;
        area.taffy_style.min_size.height = LengthPercentageAuto::length(0.0);

        let mut scroll = ResolvedStyle::default(); // .map-scroll：grow + min-height:0 + auto
        scroll.taffy_style.flex_grow = 1.0;
        scroll.taffy_style.min_size.height = LengthPercentageAuto::length(0.0);
        scroll.overflow_y = crate::style::resolved::OverflowMode::Auto;

        let mut layer = ResolvedStyle::default(); // .map-layer：显式内容尺寸
        layer.taffy_style.size.width = Dimension::length(4000.0);
        layer.taffy_style.size.height = Dimension::length(1572.0);

        let entries = [
            (None, NodeKind::Container, screen, Vec::new(), None),
            (Some(0), NodeKind::Container, topbar, Vec::new(), None),
            (Some(0), NodeKind::Container, area, Vec::new(), None),
            (Some(2), NodeKind::Container, scroll, Vec::new(), None),
            (Some(3), NodeKind::Container, layer, Vec::new(), None),
        ]
        .map(|(p, k, s, c, i)| (p, k, s, c, i, false, None, None, None, None));
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (1920.0, 1080.0),
            [0.0; 4],
            &empty_sizes(),
        );

        let area_id = scene.get(scene.roots[0]).unwrap().children[1];
        let scroll_id = scene.get(area_id).unwrap().children[0];
        let layer_id = scene.get(scroll_id).unwrap().children[0];
        let ar = scene.get(area_id).unwrap().layout_rect;
        let sr = scene.get(scroll_id).unwrap().layout_rect;
        let lr = scene.get(layer_id).unwrap().layout_rect;
        // 浏览器同构断言（Tripawd issue #64 实测预览值）：
        // - 顶栏保 76（specified-size 地板），area/scroll 钳在 flex 份额 1004；
        // - layer 保 1572 溢出（overlap=568 可滚）。修前：area/scroll 被撑到 1572
        //   （hidden 祖先触发全局 shrink=0）→ viewport==content → 不能滚。
        assert!(
            (ar.y - 76.0).abs() < 0.1,
            "顶栏保 76：area.y=76，got {}",
            ar.y
        );
        assert!(
            (ar.h - 1004.0).abs() < 0.1,
            "min-height:0 弹性份额 1004（1080-76），got {}",
            ar.h
        );
        assert!(
            (sr.h - 1004.0).abs() < 0.1,
            "滚动视口钳在 1004，got {}",
            sr.h
        );
        assert!(
            (lr.h - 1572.0).abs() < 0.1,
            "内容层保显式 1572（shrink=0 于滚动容器），got {}",
            lr.h
        );
        assert!(lr.h > sr.h, "overlap>0 才能滚");
    }

    /// #64 配套：滚动容器内容地板——显式尺寸子在滚动容器里不被 shrink 压扁
    /// （原规则的存在理由，收窄到 Auto/Scroll 后仍须成立）。
    #[test]
    fn scroll_container_children_keep_explicit_size() {
        let mut scroll = ResolvedStyle::default(); // 300px 视口 + overflow:auto
        scroll.taffy_style.flex_direction = taffy::style::FlexDirection::Column;
        scroll.taffy_style.size.width = Dimension::length(200.0);
        scroll.taffy_style.size.height = Dimension::length(300.0);
        scroll.overflow_y = OverflowMode::Auto;

        let mut filler = ResolvedStyle::default(); // .filler{height:300}
        filler.taffy_style.size.width = Dimension::length(200.0);
        filler.taffy_style.size.height = Dimension::length(300.0);

        let mut tail = ResolvedStyle::default(); // 再来 200px → 总 500 > 300
        tail.taffy_style.size.width = Dimension::length(200.0);
        tail.taffy_style.size.height = Dimension::length(200.0);

        let entries = [
            (None, NodeKind::Container, scroll, Vec::new(), None),
            (Some(0), NodeKind::Container, filler, Vec::new(), None),
            (Some(0), NodeKind::Container, tail, Vec::new(), None),
        ]
        .map(|(p, k, s, c, i)| (p, k, s, c, i, false, None, None, None, None));
        let mut scene = Scene::build(&entries);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (1920.0, 1080.0),
            [0.0; 4],
            &empty_sizes(),
        );

        let fr = scene.get(scene.roots[0]).unwrap().children[0];
        let fr = scene.get(fr).unwrap().layout_rect;
        let tr_id = scene.get(scene.roots[0]).unwrap().children[1];
        let tr2 = scene.get(tr_id).unwrap().layout_rect;
        assert!(
            (fr.h - 300.0).abs() < 0.1,
            "filler 保显式 300 不被压扁，got {}",
            fr.h
        );
        assert!(
            (tr2.y - 300.0).abs() < 0.1 && (tr2.h - 200.0).abs() < 0.1,
            "tail 紧随其后 300..500（总高 500 溢出视口），got y={} h={}",
            tr2.y,
            tr2.h
        );
    }

    /// #65 回归：Tripawd 事件卡结构（screen flex column > wrap 居中 > 卡 flex column >
    /// 17px 结果文本，CSS 写 `line-height: 27px`）。px 形经 mapping 双槽 +
    /// effective_line_height 换算后，两行文本高度 = 2×27px + 上下 padding，不再
    /// 被当 27 倍撑爆（修前单行 459px、卡片溢出屏幕）。
    #[test]
    fn text_line_height_px_wraps_to_normal_height() {
        use crate::style::mapping::apply_decl;
        use crate::style::resolved::TextAlign;
        use taffy::geometry::Rect;
        use taffy::style::LengthPercentage;

        fn pad(t: f32, r: f32, b: f32, l: f32) -> Rect<LengthPercentage> {
            Rect {
                top: LengthPercentage::length(t),
                right: LengthPercentage::length(r),
                bottom: LengthPercentage::length(b),
                left: LengthPercentage::length(l),
            }
        }

        // CSS 声明走真实 mapping 链：font-size 17px + line-height 27px。
        let mut text_style = ResolvedStyle::default();
        assert!(apply_decl(&mut text_style, "font-size", "17px"));
        assert!(apply_decl(&mut text_style, "line-height", "27px"));
        assert_eq!(text_style.line_height_px, Some(27.0), "px 形进长度槽");
        assert_eq!(text_style.line_height, 0.0, "倍数槽不动");
        text_style.text_align = TextAlign::Center;
        text_style.font_family = Some("wqy-microhei".into());
        text_style.taffy_style.padding = pad(6.0, 10.0, 6.0, 10.0);

        // 60 个 CJK 字；内容宽 600-88-20=492px，17px/字 → 两行。
        let content = "山道旁一名老者被悍匪纠缠不休你略一沉吟按剑而立贼人见状色厉内荏落荒而逃乡人皆称义士快哉快哉".to_string();

        let mut screen = ResolvedStyle::default(); // .screen：flex column 1920×1080
        screen.taffy_style.flex_direction = taffy::style::FlexDirection::Column;

        let mut wrap = ResolvedStyle::default(); // .event-wrap：grow + 双轴居中
        wrap.taffy_style.flex_grow = 1.0;
        wrap.taffy_style.align_items = Some(taffy::style::AlignItems::CENTER);
        wrap.taffy_style.justify_content = Some(taffy::style::JustifyContent::CENTER);

        let mut card = ResolvedStyle::default(); // .event-card：flex column 600px
        card.taffy_style.flex_direction = taffy::style::FlexDirection::Column;
        card.taffy_style.size.width = Dimension::length(600.0);
        card.taffy_style.padding = pad(26.0, 44.0, 22.0, 44.0);

        let entries = [
            (None, NodeKind::Container, screen, Vec::new(), None),
            (Some(0), NodeKind::Container, wrap, Vec::new(), None),
            (Some(1), NodeKind::Container, card, Vec::new(), None),
            (Some(2), NodeKind::TextNode, text_style, Vec::new(), None),
        ]
        .map(|(p, k, s, c, i)| {
            (
                p,
                k,
                s,
                c,
                i,
                false,
                None,
                None,
                Some(content.clone()),
                None,
            )
        });
        let mut scene = Scene::build(&entries);

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/wqy-microhei.ttc"
        );
        let mut fonts = FontTable::new();
        fonts
            .register("wqy-microhei", std::fs::read(path).unwrap(), true)
            .unwrap();
        solve(
            &mut scene,
            &fonts,
            (1920.0, 1080.0),
            [0.0; 4],
            &empty_sizes(),
        );

        let wrap_id = scene.get(scene.roots[0]).unwrap().children[0];
        let card_id = scene.get(wrap_id).unwrap().children[0];
        let text_id = scene.get(card_id).unwrap().children[0];
        let tr = scene.get(text_id).unwrap().layout_rect;
        let lines = scene
            .text_layouts
            .get(text_id.index())
            .cloned()
            .flatten()
            .map(|l| l.lines.len())
            .unwrap_or(0);
        assert_eq!(lines, 2, "60 字 @492px 内容宽应换两行");
        // 两行 × 27px + padding 12 = 66；修前 px 被当 27 倍 → 2×459+12 = 930。
        assert!(
            (tr.h - 66.0).abs() < 1.0,
            "两行文本高 ≈66（2×27px + padding 12），got {}",
            tr.h
        );
    }

    // ===== #29 增量 solve 差分守卫 =====
    // 增量实现的验收主体：随机操作序列下，增量 solve 与全重建 solve（solve_rebuild =
    // 清缓存重 sync）的逐节点 layout_rect/clip_rect 必须全等。漏一种 diff（结构变更/
    // 样式变更/内容变更含空白折叠/slot 复用 gen 不符/rich 折叠转正）= 旧布局残留，
    // 这里当场抓。seed 固定保证可复现。

    /// xorshift64*：测试用确定性 RNG（不引 rand 依赖）。
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545F4914F6CDD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// 全 live 节点的 (id, layout_rect, clip_rect, 文本行数) 快照，按 id 排序。
    /// 行数进快照：rect 相等但行断不同的回归（如 text_layouts 帧契约破环后的
    /// 亚像素误判换行）只有这个维度能抓。
    fn snap(scene: &Scene) -> Vec<(NodeId, Rect, Option<Rect>, usize)> {
        let mut v: Vec<_> = scene
            .nodes
            .values()
            .map(|n| {
                let lines = scene
                    .text_layouts
                    .get(n.id.index())
                    .and_then(|l| l.as_ref())
                    .map(|l| l.lines.len())
                    .unwrap_or(0);
                (n.id, n.layout_rect, n.clip_rect, lines)
            })
            .collect();
        v.sort_by_key(|(id, _, _, _)| id.0);
        v
    }

    /// 建子节点（镜像 Scene::build 的节点字面量），挂到 parent.children 尾部。
    fn spawn(scene: &mut Scene, parent: NodeId, kind: NodeKind, text: Option<String>) -> NodeId {
        let node = crate::scene::node::Node {
            id: NodeId::INVALID,
            parent: Some(parent),
            kind,
            style: ResolvedStyle::default(),
            render_input_version: 0,
            render_hidden: false,
            base_style: ResolvedStyle::default(),
            taffy_id: None,
            layout_rect: Rect::default(),
            clip_rect: None,
            children: Vec::new(),
            dirty_mesh: true,
            dirty_text: matches!(kind, NodeKind::TextNode),
            classes: Vec::new(),
            id_attr: None,
            custom_tag: None,
            interaction: crate::scene::node::NodeInteraction {
                flags: crate::scene::node::NodeFlags::empty(),
                touchable: true,
                draggable: false,
                tabindex: None,
            },
            reuse_key: 0,
            inline_override: ResolvedStyle::default(),
            inline_set: crate::style::dynamic::InlineSet(0),
            user_transform: crate::transform::NodeTransform::default(),
            rich_text_block: false,
        };
        let key = scene.nodes.insert(node);
        let id = NodeId::from_key(key);
        scene.nodes.get_mut(key).unwrap().id = id;
        scene.alloc_node_slot(id, kind);
        if let Some(t) = text {
            scene.text_contents.insert(id, t);
        }
        scene.get_live_mut(parent, "test/spawn").children.push(id);
        id
    }

    /// live 节点池（任意 kind）+ 容器池（有 children 槽位的候选）。
    fn live_nodes(scene: &Scene) -> Vec<NodeId> {
        scene.nodes.values().map(|n| n.id).collect()
    }

    #[test]
    fn incremental_solve_matches_full_rebuild_under_random_ops() {
        let Some(fonts) = font_table() else {
            // 无字体 fixture 环境（CI 极端裁剪）跳过——文本测量退 FontTable default 同样可跑，
            // 但保持与其他 layout 测试一致的跳过语义。
            return;
        };
        // 非平凡初始场景：flex column 根 + 文本/容器/图 + absolute escapee +
        // 滚动容器（高子）+ 空白文本节点（排除路径）。
        let mut abs_style = ResolvedStyle::default();
        abs_style.taffy_style.position = taffy::style::Position::Absolute;
        abs_style.position_declared = crate::style::resolved::PositionDeclared::Absolute;
        abs_style.taffy_style.inset.top = taffy::style::LengthPercentageAuto::length(10.0);
        abs_style.taffy_style.size.width = Dimension::length(80.0);
        let mut scroll_style = ResolvedStyle::default();
        scroll_style.overflow_y = OverflowMode::Scroll;
        scroll_style.taffy_style.size.height = Dimension::length(200.0);
        let mut tall_style = ResolvedStyle::default();
        tall_style.taffy_style.size.height = Dimension::length(600.0);
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(1),
                NodeKind::TextNode,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                Some("hello incremental".into()),
                None,
            ),
            (
                Some(1),
                NodeKind::TextNode,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                Some("   \n  ".into()),
                None,
            ), // 纯空白：排除路径
            (
                Some(0),
                NodeKind::Container,
                abs_style.clone(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ), // escapee：非 positioned 父
            (
                Some(0),
                NodeKind::Container,
                scroll_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(5),
                NodeKind::Container,
                tall_style,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Image,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                Some("a.png".into()),
            ),
        ];
        let mut scene = Scene::build(&entries);
        scene.image_srcs.insert(
            scene
                .nodes
                .values()
                .find(|n| n.kind == NodeKind::Image)
                .unwrap()
                .id,
            "a.png".into(),
        );
        let sizes = sizes("a.png", 120, 90);
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut tweens = crate::tween::TweenManager::default();
        let mut root_size = (800.0_f32, 600.0_f32);

        // 预热两帧（首帧全量创建，第二帧稳态复用）。
        solve(&mut scene, &fonts, root_size, [0.0; 4], &sizes);
        solve(&mut scene, &fonts, root_size, [0.0; 4], &sizes);

        for frame in 0..300 {
            // ---- 随机操作（8 类，覆盖增量 diff 的全部变更面）----
            let live = live_nodes(&scene);
            if live.len() < 4 {
                return; // 删空保护：场景太小无意义
            }
            match rng.below(8) {
                0 => {
                    // 内容变更（含偶发置空白 → 排除项转正/转出）
                    let texts: Vec<_> = scene
                        .nodes
                        .values()
                        .filter(|n| n.kind == NodeKind::TextNode)
                        .map(|n| n.id)
                        .collect();
                    let id = texts[rng.below(texts.len())];
                    let content = if rng.below(4) == 0 {
                        "  \n\t ".to_string() // 触发空白过滤路径
                    } else {
                        format!("text #{}", rng.next() % 1000)
                    };
                    scene.text_contents.insert(id, content);
                }
                1 => {
                    // 样式变更：尺寸/边距/主轴方向
                    let id = live[rng.below(live.len())];
                    let n = scene.get_live_mut(id, "test/style");
                    let r = (rng.next() % 300) as f32 + 10.0;
                    match rng.below(3) {
                        0 => n.style.taffy_style.size.width = Dimension::length(r),
                        1 => n.style.taffy_style.size.height = Dimension::length(r),
                        _ => {
                            n.style.taffy_style.padding.left =
                                taffy::style::LengthPercentage::length(r / 4.0)
                        }
                    }
                }
                2 | 3 => {
                    // 增节点（slot 复用 gen 路径：此前帧删过的槽会被重用）
                    let parents: Vec<_> = live_nodes(&scene)
                        .into_iter()
                        .filter(|id| !scene.get_live(*id, "test").rich_text_block)
                        .collect();
                    let p = parents[rng.below(parents.len())];
                    let kind = if rng.below(2) == 0 {
                        NodeKind::TextNode
                    } else {
                        NodeKind::Container
                    };
                    let text = (kind == NodeKind::TextNode).then(|| format!("spawn #{}", frame));
                    spawn(&mut scene, p, kind, text);
                }
                4 => {
                    // 删节点（随机非根；真 remove_node 走 slotmap 摘除 + 旁表清理）
                    let victim = live[1 + rng.below(live.len() - 1)];
                    crate::scene::dynamic::remove_node(&mut scene, &mut tweens, victim);
                }
                5 => {
                    // 重排：rotate 父的 children
                    let p = live[rng.below(live.len())];
                    let n = scene.get_live_mut(p, "test/reorder");
                    if n.children.len() >= 2 {
                        n.children.rotate_left(1);
                    }
                }
                6 => {
                    // 重挂：A 移到随机容器 B（防环：B 不在 A 子树内）
                    let all = live_nodes(&scene);
                    let a = all[1 + rng.below(all.len() - 1)];
                    let b = all[rng.below(all.len())];
                    let mut anc = b;
                    let mut cyclic = false;
                    loop {
                        if anc == a {
                            cyclic = true;
                            break;
                        }
                        match scene.get_live(anc, "test/reparent").parent {
                            Some(p) => anc = p,
                            None => break,
                        }
                    }
                    if !cyclic {
                        let old_parent = scene.get_live(a, "test/reparent").parent;
                        if let Some(op) = old_parent {
                            scene
                                .get_live_mut(op, "test/reparent")
                                .children
                                .retain(|c| *c != a);
                        }
                        scene.get_live_mut(a, "test/reparent").parent = Some(b);
                        scene.get_live_mut(b, "test/reparent").children.push(a);
                    }
                }
                _ => {
                    // root_size 变更（viewport 重排语言）
                    root_size = (
                        600.0 + (rng.next() % 400) as f32,
                        400.0 + (rng.next() % 300) as f32,
                    );
                }
            }
            // ---- 差分断言：增量 vs 全重建 ----
            solve(&mut scene, &fonts, root_size, [0.0; 4], &sizes);
            let a = snap(&scene);
            solve_rebuild(&mut scene, &fonts, root_size, [0.0; 4], &sizes);
            let b = snap(&scene);
            assert_eq!(
                a, b,
                "frame {frame}: incremental solve diverged from full rebuild"
            );
        }
    }

    /// text_layouts 的跨帧契约（#29 增量 + 换行回归）：稳态帧 taffy 缓存全命中、measure
    /// 闭包不跑，text_layouts 必须承接上帧——否则 render 退回整数化 content_w 重测，
    /// 短文本因 intrinsic 亚像素超宽误判换行（「首页/叠/内」末字被挤下行的真机病灶）。
    #[test]
    fn steady_frame_preserves_text_layouts_for_render() {
        let Some(fonts) = font_table() else { return };
        let entries = [(
            None,
            NodeKind::Container,
            ResolvedStyle::default(),
            Vec::new(),
            None,
            false,
            None,
            None,
            None,
            None,
        )];
        let mut scene = Scene::build(&entries);
        let root = scene.roots[0];
        let t = spawn(
            &mut scene,
            root,
            NodeKind::TextNode,
            Some("短文本宽度贴边".into()),
        );
        let sizes = empty_sizes();
        solve(&mut scene, &fonts, (800.0, 600.0), [0.0; 4], &sizes);
        assert!(
            scene.text_layouts[t.index()].is_some(),
            "首帧 measure 必跑、render 槽必填"
        );
        solve(&mut scene, &fonts, (800.0, 600.0), [0.0; 4], &sizes); // 稳态帧：零变更
        assert!(
            scene.text_layouts[t.index()].is_some(),
            "稳态帧 taffy 跳过干净子树，text_layouts 必须承接上帧（None = render 退化重测换行）"
        );
    }

    // —— #10 layout 动画 override 覆写链 ——

    #[test]
    fn anim_width_height_override_drives_solve() {
        // anim.width/height（px 域）覆写 base 声明：solve 读到的是 override 值。
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        let child = scene.get(scene.roots[0]).unwrap().children[0];
        let id = scene.get(child).unwrap().id;
        scene.anim.ensure(id).width = Some(crate::scene::AnimLen {
            domain: crate::scene::LenDomain::Px,
            value: 123.0,
        });
        scene.anim.ensure(id).height = Some(crate::scene::AnimLen {
            domain: crate::scene::LenDomain::Px,
            value: 45.0,
        });
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (800.0, 600.0),
            [0.0; 4],
            &HashMap::new(),
        );
        let r = &scene.get(id).unwrap().layout_rect;
        assert!(
            (r.w - 123.0).abs() < 0.1,
            "anim width override 123, got {}",
            r.w
        );
        assert!(
            (r.h - 45.0).abs() < 0.1,
            "anim height override 45, got {}",
            r.h
        );
    }

    #[test]
    fn anim_vw_width_reresolves_on_resize_mid_flight() {
        // resize mid-flight（#10 决策）：vw 域动画中途 root_size 变 → 下帧 solve 按新
        // root_size 重解析（动画继续走完、比例自动跟随画布，不 snap 不重启）。
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        let child = scene.get(scene.roots[0]).unwrap().children[0];
        let id = scene.get(child).unwrap().id;
        // 动画进行中：50vw（progress 已到该值，tween 还在跑）
        scene.anim.ensure(id).width = Some(crate::scene::AnimLen {
            domain: crate::scene::LenDomain::Vw,
            value: 50.0,
        });
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (800.0, 600.0),
            [0.0; 4],
            &HashMap::new(),
        );
        let w1 = scene.get(id).unwrap().layout_rect.w;
        assert!((w1 - 400.0).abs() < 0.1, "50vw @800 -> 400, got {w1}");
        // resize 到 1000：同 override 值（动画未推进），重 solve 跟随新画布
        solve(
            &mut scene,
            &fonts,
            (1000.0, 500.0),
            [0.0; 4],
            &HashMap::new(),
        );
        let w2 = scene.get(id).unwrap().layout_rect.w;
        assert!(
            (w2 - 500.0).abs() < 0.1,
            "50vw @1000 -> 500（重解析跟随），got {w2}"
        );
    }

    #[test]
    fn anim_flex_grow_override_shares_space() {
        // flex-grow override：兄弟份额换手（侧栏收起动画的 solve 消费证据）。
        use crate::style::mapping::apply_decl;
        let mut st = ResolvedStyle::default();
        assert!(apply_decl(&mut st, "flex-grow", "1"));
        let st2 = st.clone();
        let entries = [
            (
                None,
                NodeKind::Container,
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Container,
                st,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
            (
                Some(0),
                NodeKind::Container,
                st2,
                Vec::new(),
                None,
                false,
                None,
                None,
                None,
                None,
            ),
        ];
        let mut scene = Scene::build(&entries);
        let root = scene.roots[0];
        let kids = scene.get(root).unwrap().children.clone();
        // 左子动画 override flex-grow = 0（收起中）
        scene.anim.ensure(kids[0]).flex_grow = Some(0.0);
        let fonts = font_table().expect("need font");
        solve(
            &mut scene,
            &fonts,
            (800.0, 600.0),
            [0.0; 4],
            &HashMap::new(),
        );
        let lw = scene.get(kids[0]).unwrap().layout_rect.w;
        let rw = scene.get(kids[1]).unwrap().layout_rect.w;
        assert!(lw.abs() < 0.1, "grow=0 的子收缩到 0，got {lw}");
        assert!((rw - 800.0).abs() < 0.1, "另一子占满，got {rw}");
    }
}
