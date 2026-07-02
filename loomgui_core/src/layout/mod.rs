//! Layout 层：taffy 集成。
//!
//! 消费 `Scene`（Node 树 + `ResolvedStyle`），建 taffy 树，注册叶子节点的
//! 测量上下文（Text/Image），solve 后把 taffy 的 `Layout.location`/`size`
//! 回写进 `Node.layout_rect`/`clip_rect`。
//!
//! # taffy 0.5.2 API 边界
//!
//! taffy 0.5.2 用 trait 对象模式（无 `MeasureFunc` 枚举）：
//! - `TaffyTree<NodeContext>`：节点上下文是泛型，叶子节点用
//!   `new_leaf_with_context(style, ctx)` 存一个 owned `NodeContext`。
//! - 单个 `compute_layout_with_measure(root, avail, FnMut(...))` 闭包负责按
//!   `Option<&mut NodeContext>` 分派到 Text/Image 测量。
//!
//! 测量是单个 `FnMut`（非 'static），生命周期与 `compute_layout_with_measure`
//! 调用同界——闭包内借 `&font` 合法。每个叶子的文本参数（content/font_size 等）已
//! owned 进 `NodeContext::Text`，font 不进 context 而走闭包借用。`solve` 签名
//! 收 `font: &Font`（不破下游 stage 契约）。
//!
//! taffy 0.5.2 的 `Style` 无 `order`，不做 flex order 排序（render 层按 DOM 顺序 /
//! layout 输出的 `Layout.order` 渲染）。
//!
//! v1.4-a D17：核心知图尺寸（打包期 PNG IHDR 静态，Stage 持 path→(w,h) 尺寸表）+ 不知图集
//! （运行时纹理/UV 归 Unity）。solve 接 `image_sizes: &HashMap<String,(u32,u32)>` 查 Image intrinsic
//! 尺寸（三档：CSS > 真实像素 > 64×64）。render payload 带 path，UV 全图 (0,0)-(1,1)。

use crate::scene::node::{NodeId, NodeKind, Rect, Scene};
use crate::style::resolved::{OverflowMode, TextAlign};
use crate::text::layout::{measure_text, Font, TextLayout};
use std::collections::HashMap;
use taffy::prelude::*;

/// D17 图尺寸表类型别名：归一化 path → (w, h) 像素（打包期 PNG IHDR 静态）。
/// `solve`/`build_render_nodes` 接 `&HashMap<String, (u32, u32)>` 查 Image intrinsic 尺寸。
pub type ImageSizeTable = HashMap<String, (u32, u32)>;

/// LoomGUI OverflowMode → taffy Overflow（Auto→Scroll，taffy 0.5 无 Auto）。
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

/// 叶子节点的测量上下文。Container/Button 无上下文（用 None 叶子或 new_with_children）。
enum MeasureContext {
    /// Text 叶子：存全部测量参数（content owned）+ 字体度量字段。
    /// font *不* 进 context——font 在闭包借用层共享（见模块 doc）。
    Text {
        content: String,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        align: TextAlign,
        nowrap: bool,
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

/// 就地 solve：建 taffy 树 → 注册测量上下文 → compute_layout → 回写 layout_rect/clip_rect。
///
/// `root_size` 是根节点固定尺寸（viewport / surface 尺寸）。`font` 借用到
/// `compute_layout_with_measure` 结束，闭包内解引用喂给 `measure_text`。
///
/// **D17**：`image_sizes` = Stage 持有的 path→(w,h) 尺寸表（打包期 PNG IHDR 静态）。
/// Image measure 查此表算 intrinsic 尺寸（三档：CSS > 真实像素 > 64×64）。
/// path 缺失或 w/h=0 → fallback 64×64（核心不知图集，但知图尺寸）。
pub fn solve(scene: &mut Scene, font: &Font, root_size: (f32, f32), image_sizes: &ImageSizeTable) {
    // 防御：空 roots（空 scene）无几何可 solve——直接返回，避免 roots[0] 越界 panic。
    // Stage 可能在 scene 未装内容时 tick（如测/边界），不应 panic。
    if scene.roots.is_empty() {
        return;
    }
    let mut taffy_tree: TaffyTree<MeasureContext> = TaffyTree::new();
    // scene NodeId → taffy NodeId 映射（按 NodeId.index() 索引，1 基故 capacity+1）。
    // **容量而非存活数**（T5）：remove_node 后 slotmap idx 不变但存活数减——按 len 分配会越界。
    let mut taffy_ids: Vec<Option<taffy::NodeId>> = vec![None; scene.nodes.capacity() + 1];

    fn build(
        scene: &Scene,
        tree: &mut TaffyTree<MeasureContext>,
        taffy_ids: &mut Vec<Option<taffy::NodeId>>,
        id: NodeId,
        parent_overflow: bool,
        image_sizes: &ImageSizeTable,
    ) -> taffy::NodeId {
        let node = scene.get(id).expect("live node");
        let mut style = node.style.taffy_style.clone();
        // overflow != visible → 设 taffy overflow，让 flex automatic min-size=0（CSS flex §4.5）。
        // 不设则 taffy 默认 Visible → min-size=min-content → 容器被 content 撑开（viewport=content）
        // → overlap=0 → scroll 失效。
        style.overflow = taffy::geometry::Point {
            x: map_overflow(node.style.overflow_x),
            y: map_overflow(node.style.overflow_y),
        };
        // overflow 容器的直接子 flex-shrink=0：保持显式尺寸/min-content 溢出（scroll 有效）。
        // 否则空内容子（如 .filler{height:300} min-content=0）被 shrink 到 viewport → overlap=0 → 不能滚。
        if parent_overflow {
            style.flex_shrink = 0.0;
        }
        let self_overflow = node.style.overflow_x != OverflowMode::Visible
            || node.style.overflow_y != OverflowMode::Visible;
        // 叶子：Text/Image 装 MeasureContext。
        let ctx: Option<MeasureContext> = match &node.kind {
            NodeKind::Text { content } => {
                let s = &node.style;
                Some(MeasureContext::Text {
                    content: content.clone(),
                    font_size: s.font_size,
                    line_height: s.line_height,
                    letter_spacing: s.letter_spacing,
                    align: s.text_align,
                    nowrap: s.white_space_nowrap,
                })
            }
            NodeKind::Image { src } => {
                // D17：查 Stage 尺寸表算 intrinsic 尺寸（三档：CSS > 真实像素 > 64×64）。
                // path 缺失或 w/h=0 → fallback 64×64。核心不知图集（运行时纹理归 Unity）。
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
        };

        // 递归子节点（先建子，再建父以便 new_with_children）。
        let children_ids: Vec<taffy::NodeId> = node
            .children
            .iter()
            .map(|c| build(scene, tree, taffy_ids, *c, self_overflow, image_sizes))
            .collect();

        let tid = if let Some(mctx) = ctx {
            // 叶子：装测量上下文。children 应为空（Text/Image 是叶子）。
            tree.new_leaf_with_context(style, mctx).unwrap()
        } else {
            // 容器：用 children 建。
            tree.new_with_children(style, &children_ids).unwrap()
        };
        taffy_ids[id.index()] = Some(tid);
        tid
    }

    let root_tid = build(scene, &mut taffy_tree, &mut taffy_ids, scene.roots[0], false, image_sizes);

    // taffy NodeId → scene NodeId 反查，供 measure 闭包按 taffy nid 把 TextLayout
    // 存进 scene 索引的 text_layouts。render 复用，消除 layout/render 双测量不一致。
    let mut taffy_to_scene: HashMap<taffy::NodeId, NodeId> = HashMap::new();
    for n in scene.nodes.values() {
        if let Some(tid) = taffy_ids[n.id.index()] {
            taffy_to_scene.insert(tid, n.id);
        }
    }
    let mut text_layouts: Vec<Option<TextLayout>> = vec![None; scene.nodes.capacity() + 1];

    // 设根 size：覆盖为调用方给的 root_size（viewport）。
    // Style.size 字段类型是 Size<Dimension>（不是 LengthPercentageAuto）。
    let root_style = taffy_tree.style(root_tid).unwrap().clone();
    taffy_tree
        .set_style(
            root_tid,
            Style {
                size: Size {
                    width: Dimension::Length(root_size.0),
                    height: Dimension::Length(root_size.1),
                },
                ..root_style
            },
        )
        .ok();

    // solve：单一 FnMut 闭包按 context 分派。
    // known.width: Option<f32> —— Some=约束宽，None=不限（→ measure_text max_width=None）。
    taffy_tree
        .compute_layout_with_measure(
            root_tid,
            Size::MAX_CONTENT,
            |known: Size<Option<f32>>,
             _avail: Size<AvailableSpace>,
             nid: taffy::NodeId,
             node_ctx: Option<&mut MeasureContext>,
             _style: &Style|
             -> Size<f32> {
                match node_ctx {
                    None => Size::ZERO,
                    Some(MeasureContext::Image { iw, ih, w_dim, h_dim }) => {
                        let (iw, ih, wd, hd) = (*iw, *ih, *w_dim, *h_dim);
                        // width：known.width（Percent/fit 解析后，taffy 传）> css Length > 等比 height > intrinsic。
                        //   Percent width：taffy 第二次传 known.width=Some(解析宽)。
                        let w = match (known.width, wd, hd) {
                            (Some(v), _, _) => v,
                            (None, Dimension::Length(v), _) => v,
                            (None, Dimension::Auto, Dimension::Length(h)) => h * iw / ih,
                            (None, _, _) => iw,
                        };
                        // height：css Length > known.height > 等比 width（CSS img height:auto 默认）。
                        let h = match (hd, known.height) {
                            (Dimension::Length(v), _) => v,
                            (_, Some(v)) => v,
                            _ => w * ih / iw,
                        };
                        Size { width: w, height: h }
                    }
                    Some(MeasureContext::Text { content, font_size, line_height, letter_spacing, align, nowrap }) => {
                        let layout = measure_text(
                            content,
                            *font_size,
                            *line_height,
                            *letter_spacing,
                            *align,
                            *nowrap,
                            known.width,
                            font,
                        );
                        // 存 TextLayout 供 render 复用。Some（available 测量）优先——
                        // 短文本 taffy 只传 None（max-content ≤ available，不换行），长文本传
                        // Some(available)（换行）。一旦存了 Some，后续 None 不覆盖（taffy 末尾
                        // 可能补测 None）。
                        if let Some(sid) = taffy_to_scene.get(&nid) {
                            let slot = &mut text_layouts[sid.index()];
                            if slot.is_none() || known.width.is_some() {
                                *slot = Some(layout.clone());
                            }
                        }
                        Size { width: layout.text_width, height: layout.text_height }
                    }
                }
            },
        )
        .ok();

    // 回写 layout_rect + clip_rect（递归累加父 origin 得绝对坐标）。
    fn write_back(
        scene: &mut Scene,
        tree: &TaffyTree<MeasureContext>,
        taffy_ids: &[Option<taffy::NodeId>],
        id: NodeId,
        parent_origin: (f32, f32),
    ) {
        let tid = taffy_ids[id.index()].unwrap();
        let layout = tree.layout(tid).unwrap();
        let x = parent_origin.0 + layout.location.x;
        let y = parent_origin.1 + layout.location.y;
        let (w, h) = (layout.size.width, layout.size.height);
        let node = scene.get_mut(id).expect("live node");
        node.layout_rect = Rect { x, y, w, h };
        // overflow:hidden 节点（build_scene 已建 Some 槽）：用自身 border 框填 clip。
        if node.clip_rect.is_some() {
            node.clip_rect = Some(Rect { x, y, w, h });
        }
        let kids = node.children.clone();
        for c in kids {
            write_back(scene, tree, taffy_ids, c, (x, y));
        }
    }
    write_back(scene, &taffy_tree, &taffy_ids, scene.roots[0], (0.0, 0.0));
    // layout 阶段 TextLayout 缓存交还 scene，供 render 复用（不重测）。
    scene.text_layouts = text_layouts;
}

#[cfg(all(test, feature = "parse"))]
mod tests {
    use super::*;
    use crate::parse::{css::parse_css, dom::parse_html};
    use crate::scene::{build_scene, NodeKind, Scene};
    use crate::style::cascade::resolve_styles;
    use crate::style::resolved::ResolvedStyle;

    fn font() -> Option<Font> {
        let p = format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"));
        Font::from_path(&p).ok()
    }

    /// D17 测试辅助：空图尺寸表（无 path → 全 64×64 兜底）。
    fn empty_sizes() -> ImageSizeTable {
        HashMap::new()
    }

    /// D17 测试辅助：建单条 path→(w,h) 尺寸表。
    fn sizes(path: &str, w: u32, h: u32) -> ImageSizeTable {
        let mut m = HashMap::new();
        m.insert(path.to_string(), (w, h));
        m
    }

    #[test]
    fn column_stack_sizes_children() {
        let html2 = r#"<div class="root"><div class="a"></div><div class="b"></div></div>"#;
        // div 默认 flex-direction: column（ResolvedStyle::default 落地）。
        // CSS 不写 flex-direction，子项也应垂直堆叠。
        let css = r#".root { width: 200px; height: 200px; } .a { height: 50px; } .b { height: 30px; }"#;
        let tree = parse_html(html2).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let mut scene = build_scene(&tree, &styles);
        solve(&mut scene, &font().expect("test needs a font"), (200.0, 200.0), &empty_sizes());
        let root = scene.get(scene.roots[0]).unwrap();
        let a = scene.get(root.children[0]).unwrap();
        let b = scene.get(root.children[1]).unwrap();
        assert!((a.layout_rect.h - 50.0).abs() < 0.1);
        assert!((b.layout_rect.h - 30.0).abs() < 0.1);
        assert!((b.layout_rect.y - 50.0).abs() < 0.1); // 垂直堆叠
    }

    /// 回归：未显式写 flex-direction 的 div 默认垂直堆叠（column）。
    /// 防止有人把 ResolvedStyle::default 的 flex_direction 改回 Row。
    #[test]
    fn default_div_is_column() {
        let html = r#"<div class="root"><div class="a"></div><div class="b"></div></div>"#;
        let css = r#".root { width: 200px; height: 200px; } .a { width: 50px; height: 50px; } .b { width: 30px; height: 30px; }"#;
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = resolve_styles(&tree, &sheet);
        let mut scene = build_scene(&tree, &styles);
        solve(&mut scene, &font().expect("test needs a font"), (200.0, 200.0), &empty_sizes());
        let root = scene.get(scene.roots[0]).unwrap();
        let a = scene.get(root.children[0]).unwrap();
        let b = scene.get(root.children[1]).unwrap();
        // 垂直堆叠：b.y ≈ a.h（a 在上，b 在下）。
        // 若默认回退到 row，b.y 会 ≈ 0（横排），此断言失败。
        assert!(
            (b.layout_rect.y - a.layout_rect.h).abs() < 0.1,
            "expected column stack (b.y ≈ a.h), got a.h={} b.y={}",
            a.layout_rect.h,
            b.layout_rect.y
        );
        // a、b 同 x（列内左对齐），x 都 ≈ 0。
        assert!(a.layout_rect.x.abs() < 0.1);
        assert!(b.layout_rect.x.abs() < 0.1);
    }

    /// Image measure 三档优先级（CSS Length > 真实像素 > 64×64 兜底）。
    /// 用 Scene::build 手搓 Image scene，不走 parse_html。
    ///
    /// **布局陷阱**：`solve` 会用 `root_size` 覆盖根节点的 taffy size（见 prod
    /// `set_style(... size: Length(root_size) ...)`），故 Image 不能做根——否则
    /// 其 MeasureContext 的 intrinsic 尺寸被 root_size 强制覆盖，测不出三档。
    /// 包一层 Container 根（idx 0），Image 做 leaf 子（idx 1），其 measure 值才生效。
    #[test]
    fn image_css_length_overrides_intrinsic() {
        // CSS width:100px height:50px → CSS 声明赢（覆盖 intrinsic 真实像素 / 64×64 兜底）。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.size.width = Dimension::Length(100.0);
        img_style.taffy_style.size.height = Dimension::Length(50.0);
        let entries = [
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Image { src: "x.png".into() }, img_style, Vec::new(), None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        solve(&mut scene, &font().expect("need font"), (300.0, 300.0), &sizes("x.png", 40, 20));
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect; // Image 是 root 唯一子
        assert!((r.w - 100.0).abs() < 0.1, "CSS length 赢：w=100，got {}", r.w);
        assert!((r.h - 50.0).abs() < 0.1, "CSS length 赢：h=50，got {}", r.h);
    }

    /// D17 恢复：无 CSS 尺寸 → 用尺寸表真实像素（40×20）。
    /// T6 曾把此测改 64×64 兜底（"核心不知图尺寸"误判）；D17 修正为核心知图尺寸。
    #[test]
    fn image_measure_uses_real_dims_when_no_css() {
        // 无 CSS 尺寸 + 尺寸表有 x.png=40×20 → intrinsic = 40×20（真实像素）。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.align_self = Some(AlignSelf::FlexStart);
        let entries = [
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Image { src: "x.png".into() }, img_style, Vec::new(), None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        solve(&mut scene, &font().expect("need font"), (300.0, 300.0), &sizes("x.png", 40, 20));
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect; // Image 是 root 唯一子
        assert!((r.w - 40.0).abs() < 0.1, "真实像素：w=40，got {}", r.w);
        assert!((r.h - 20.0).abs() < 0.1, "真实像素：h=20，got {}", r.h);
    }

    /// D17：无 CSS + 尺寸表无 path / w,h=0 → 64×64 兜底（三档第三档）。
    #[test]
    fn image_measure_uses_64_fallback_when_no_size_entry() {
        // 无 CSS + 尺寸表无 x.png → 64×64 兜底。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.align_self = Some(AlignSelf::FlexStart);
        let entries = [
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Image { src: "x.png".into() }, img_style, Vec::new(), None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        solve(&mut scene, &font().expect("need font"), (300.0, 300.0), &empty_sizes());
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        assert!((r.w - 64.0).abs() < 0.1, "兜底：w=64，got {}", r.w);
        assert!((r.h - 64.0).abs() < 0.1, "兜底：h=64，got {}", r.h);
    }

    /// D17：尺寸表 w/h=0（非 PNG / 读失败）→ fallback 64×64。
    #[test]
    fn image_measure_falls_back_to_64_when_zero_dims() {
        // 尺寸表 x.png=(0,0)（非 PNG 兜底）→ fallback 64×64。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.align_self = Some(AlignSelf::FlexStart);
        let entries = [
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Image { src: "x.png".into() }, img_style, Vec::new(), None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        solve(&mut scene, &font().expect("need font"), (300.0, 300.0), &sizes("x.png", 0, 0));
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        assert!((r.w - 64.0).abs() < 0.1, "w/h=0 → 兜底 w=64，got {}", r.w);
        assert!((r.h - 64.0).abs() < 0.1, "w/h=0 → 兜底 h=64，got {}", r.h);
    }

    /// D17 恢复：img style="width:80px" + 真实 40×20 → height 等比 = 40（80×20/40，2:1 aspect）。
    /// T6 曾改断言 h=80（64×64 1:1 兜底）；D17 修正为真实 2:1 aspect。
    #[test]
    fn image_measure_scales_height_to_width_aspect() {
        // img style="width:80px" intrinsic 40×20（真实，2:1）→ height 等比 = 40（80×20/40）。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.size.width = Dimension::Length(80.0);
        img_style.taffy_style.align_self = Some(AlignSelf::FlexStart);
        let entries = [
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Image { src: "x.png".into() }, img_style, Vec::new(), None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        solve(&mut scene, &font().expect("need font"), (300.0, 300.0), &sizes("x.png", 40, 20));
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        assert!((r.w - 80.0).abs() < 0.1, "w=80 (CSS)");
        assert!((r.h - 40.0).abs() < 0.1, "h 等比=40（80×20/40，2:1 真实 aspect），got {}", r.h);
    }

    /// D17 恢复：img style="height:60px" + 真实 40×20 → width 等比 = 120（60×40/20，2:1 aspect）。
    #[test]
    fn image_measure_scales_width_to_height_aspect() {
        // 只设 height：style="height:60px" intrinsic 40×20（真实，2:1）→ width 等比 = 120（60×40/20）。
        let mut img_style = ResolvedStyle::default();
        img_style.taffy_style.size.height = Dimension::Length(60.0);
        img_style.taffy_style.align_self = Some(AlignSelf::FlexStart);
        let entries = [
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Image { src: "x.png".into() }, img_style, Vec::new(), None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        solve(&mut scene, &font().expect("need font"), (300.0, 300.0), &sizes("x.png", 40, 20));
        let img_id = scene.get(scene.roots[0]).unwrap().children[0];
        let r = &scene.get(img_id).unwrap().layout_rect;
        assert!((r.h - 60.0).abs() < 0.1, "h=60 (CSS)");
        assert!((r.w - 120.0).abs() < 0.1, "w 等比=120（60×40/20，2:1 真实 aspect），got {}", r.w);
    }
}
