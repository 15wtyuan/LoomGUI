//! Render 层入口：遍历 solve 后的 Scene → `Vec<RenderNode>`。
//!
//! 顺序与 `scene.nodes` 索引一致（便于 node_id 对齐），payload 按 kind 决定：
//! - Container/Button → Mesh quad（背景色；无背景色时透明）
//! - Image → Mesh quad + tex_id（注册表查，未注册=0 哨兵→后端白占位）
//! - Text → measure_text 产 TextLayout，装 Text payload
//!
//! 最后调 `batch::assign_sort_keys` 填 sort_key + mask_context。

pub mod batch;
pub mod dirty;   // dirty hash（逐节点 → u64，跨帧比决定 Unchanged emit）
pub mod merge;
pub mod mesh;
pub mod node;

use crate::asset::texture::{TexMeta, TextureRegistry};
use crate::scene::node::{NodeId, NodeKind, Rect, Scene};
use crate::text::layout::{measure_text, Font};
use node::*;

use taffy::style::LengthPercentage;

/// clip 表条目：context_id（mask_context>0 的层级）→ 该层级的交集绝对 design rect。
///
/// 由 `batch::assign_sort_keys` 在 DFS 时产；`context_id` 与 RenderNode 的
/// `mask_context.0` 对齐（被该 clip 约束的节点引用同一 id）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipEntry {
    pub context_id: u32,
    pub rect: Rect,
}

/// 一帧渲染数据：节点 + clip 表（FFI blob 同帧 emit）。
///
/// `clips` 只含 mask_context>0 的层级；context==0（无 clip）永不入表。
/// 由 `build_render_nodes` 产，`stage::tick_and_render` 透传，`blob::build_blob` 消费。
#[derive(Debug, Clone)]
pub struct FrameData {
    pub nodes: Vec<RenderNode>,
    pub clips: Vec<ClipEntry>,
}

/// 构造合成 scrollbar thumb RenderNode。
/// node_id=sentinel (container|flag)，world_matrix=IDENTITY (design 绝对坐标)，
/// mask_context=0 (不裁剪)，半透明灰 quad。
fn thumb_render_node(node_id: u32, rect: Rect, sort_key: u32) -> RenderNode {
    let (v, uvc, col, idx) =
        crate::render::mesh::quad(&rect, [0.6, 0.6, 0.6, 0.6], [0.0, 0.0], [1.0, 1.0]);
    RenderNode {
        node_id,
        parent_id: None,
        visible: true,
        alpha: 1.0,
        grayed: false,
        color_tint: [1.0, 1.0, 1.0, 1.0],
        world_matrix: crate::transform::IDENTITY,
        blend: BlendMode::Normal,
        mask_context: MaskContext(0),
        sort_key,
        payload: NodePayload::Mesh {
            verts: v,
            uvs: uvc,
            colors: col,
            indices: idx,
            texture: 0,
            program: 0,
            color_matrix: [0.0; 20],
        },
    }
}

/// 遍历 Scene → `FrameData`（nodes + clip 表）。
///
/// 顺序与 `scene.nodes` 同序（node_id == scene 索引），便于 batch DFS 对齐。
/// Text 节点调 `measure_text` 产 TextLayout；Container/Image 产 Mesh quad。
/// `font` 仅 Text 节点用（单字体）。clip 表由 `batch::assign_sort_keys` 算
/// 祖先 clip 链交集后产出。
pub fn build_render_nodes(
    scene: &Scene,
    font: &Font,
    textures: &TextureRegistry,
    prev_hashes: &[u64],
) -> (FrameData, Vec<u64>) {
    let n_nodes = scene.nodes.len();
    // nodes/new_hashes 按 scene.nodes.values() 0 基顺序索引（FrameData 输出 0 基，
    // 不改 FFI 契约）。NodeId → 0 基位置映射用 slotmap 插入序（= values() 顺序）。
    let id_to_pos: std::collections::HashMap<NodeId, usize> = scene
        .nodes
        .values()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    // 预分配 Unchanged 占位，逐个按 scene 节点覆写。
    let mut nodes: Vec<RenderNode> = (0..n_nodes)
        .map(|_| RenderNode {
            node_id: 0,
            parent_id: None,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            world_matrix: crate::transform::IDENTITY,
            blend: BlendMode::Normal,
            mask_context: MaskContext(0),
            sort_key: 0,
            payload: NodePayload::Unchanged,
        })
        .collect();
    // 本帧每节点的新 hash（emit 后算）。
    let mut new_hashes: Vec<u64> = vec![0; n_nodes];
    let baselined = prev_hashes.len() == n_nodes;

    for n in scene.nodes.values() {
        let pos = id_to_pos[&n.id];
        let rn = &mut nodes[pos];
        let anim = scene.anim.get(n.id);
        rn.node_id = n.id.0 as u32;
        rn.parent_id = n.parent.map(|p| p.0 as u32);
        rn.alpha = anim.and_then(|a| a.opacity).unwrap_or(n.style.opacity);
        rn.color_tint = anim.and_then(|a| a.text_color).unwrap_or(n.style.color);
        // world_transforms 1 基索引（transform.rs 按 id.index() 填，len=N+1）。
        let wm = scene.world_transforms[n.id.index()];
        rn.world_matrix = wm;
        rn.visible = true;
        let rect = if crate::transform::is_pure_translation(&wm) {
            // scroll：world.tx 含 scroll offset（world = T(layout−scroll)）。
            // rect 用 world.tx 位置 → quad 产 world 位置 vert → blob re-base 减 world.tx → 正好 top-local
            // → MirrorPool GO at world.tx → 渲染 = world.tx = layout−scroll（scroll 跟随）。
            // 无 scroll 时 world.tx=layout → 零回归。
            crate::scene::node::Rect { x: wm[4], y: wm[5], w: n.layout_rect.w, h: n.layout_rect.h }
        } else {
            crate::scene::node::Rect { x: 0.0, y: 0.0, w: n.layout_rect.w, h: n.layout_rect.h }
        };
        let rect = &rect;
        match &n.kind {
            NodeKind::Container | NodeKind::Button => {
                let color = anim.and_then(|a| a.bg_color)
                    .unwrap_or(n.style.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]));
                let (texture, u_min, u_max, src_w, src_h) = match &n.style.background_image {
                    Some(url) => match textures.get(url) {
                        Some(m) => {
                            let (a, b) = fit_uv(n.style.background_size, rect, &m);
                            (m.tex_id, a, b, m.width as f32, m.height as f32)
                        }
                        None => (0u32, [0.0, 0.0], [1.0, 1.0], 1.0, 1.0),  // 未注册哨兵→白占位
                    },
                    None => (0u32, [0.0, 0.0], [1.0, 1.0], 1.0, 1.0),  // 无图：UV 无意义
                };
                // border-radius resolve：CSS 原始值 → 像素（% 乘 rect 对应边）
                let resolve = |lp: LengthPercentage, side: f32| -> f32 {
                    match lp {
                        LengthPercentage::Length(v) => v,
                        LengthPercentage::Percent(p) => side * p,
                    }
                };
                let (rw, rh) = (rect.w, rect.h);
                let bc = &n.style.border_radius.corners;
                let radii = [
                    (resolve(bc[0].h, rw), resolve(bc[0].v, rh)),  // TL
                    (resolve(bc[1].h, rw), resolve(bc[1].v, rh)),  // TR
                    (resolve(bc[2].h, rw), resolve(bc[2].v, rh)),  // BR
                    (resolve(bc[3].h, rw), resolve(bc[3].v, rh)),  // BL
                ];
                let all_zero = radii.iter().all(|&(rx, ry)| rx <= 0.0 || ry <= 0.0);
                // v 翻转（同 Image 分支：design y-down 配 Unity y-up；
                // 交换 uv v 传给 mesh 函数，所有 mesh 函数同样处理）
                let has_slice = n.style.border_image_slice.is_some();
                // contain 缩 geometry（左上 CSS position 0% 0%）；slice+contain 不组合（showcase 无）。
                let draw_rect = if !has_slice
                    && matches!(n.style.background_size, crate::style::resolved::BackgroundSize::Contain)
                {
                    let s = (rect.w / src_w.max(1.0)).min(rect.h / src_h.max(1.0));
                    crate::scene::node::Rect { x: rect.x, y: rect.y, w: src_w.max(1.0) * s, h: src_h.max(1.0) * s }
                } else {
                    *rect
                };
                let (v, uvc, col, idx) = match (has_slice, all_zero) {
                    (false, true)  => crate::render::mesh::quad(
                        &draw_rect, color, [u_min[0], u_max[1]], [u_max[0], u_min[1]],
                    ),
                    (false, false) => crate::render::mesh::rounded_rect(
                        &draw_rect, color, &radii,
                        [u_min[0], u_max[1]], [u_max[0], u_min[1]],
                    ),
                    (true,  true)  => crate::render::mesh::nine_slice(
                        rect, color, n.style.border_image_slice.as_ref().unwrap(),
                        src_w, src_h,
                        [u_min[0], u_max[1]], [u_max[0], u_min[1]],
                    ),
                    (true,  false) => crate::render::mesh::nine_slice_rounded(
                        rect, color, n.style.border_image_slice.as_ref().unwrap(),
                        &radii, src_w, src_h,
                        [u_min[0], u_max[1]], [u_max[0], u_min[1]],
                    ),
                };
                // program：有 color_filter → 3 或 4（叠加 filter，mesh 几何不变）；
                //   4=filter+bg-image（COLOR_FILTER+BG_COMPOSITE 双 keyword，spec §3.2 禁用皮肤按钮核心用例）；
                //   3=filter 无 bg-image（COLOR_FILTER only，base tex*vcol）。
                // 否则 bg-image 命中纹理（tex_id≠0）→ 2（CSS 合成，坑 79）；
                // 否则 0（tex*vcol：无图白占位×bg-color=bg-color，未注册哨兵同）。
                let has_filter = n.style.color_filter.is_some();
                let program = if has_filter {
                    if texture != 0 { 4u32 } else { 3u32 }   // 4=bg-image+filter 双 keyword, 3=filter only
                } else if texture != 0 { 2u32 } else { 0u32 };
                let color_matrix = n.style.color_filter.unwrap_or([0.0; 20]);
                rn.payload = NodePayload::Mesh {
                    verts: v, uvs: uvc, colors: col, indices: idx, texture, program, color_matrix,
                };
            }
            NodeKind::Image { src } => {
                let (tex_id, uv_min, uv_max, src_w, src_h) = match textures.get(src) {
                    Some(m) => (m.tex_id, m.uv_min, m.uv_max, m.width as f32, m.height as f32),
                    None => (0u32, [0.0, 0.0], [1.0, 1.0], 1.0, 1.0),
                };
                // v 翻转：design y-down + LoomStage scale (sf,-sf,sf) 把 design 顶映到屏幕上；
                // 所有 mesh 函数 TL→传入的umin/vmin，交换 v 后 TL→(umin, vmax)（texture 顶）。
                let (v, uvc, col, idx) = match &n.style.border_image_slice {
                    Some(slice) => crate::render::mesh::nine_slice(
                        rect, [1.0; 4], slice, src_w, src_h,
                        [uv_min[0], uv_max[1]], [uv_max[0], uv_min[1]],
                    ),
                    None => crate::render::mesh::quad(
                        rect, [1.0, 1.0, 1.0, 1.0],
                        [uv_min[0], uv_max[1]], [uv_max[0], uv_min[1]],
                    ),
                };
                let has_filter = n.style.color_filter.is_some();
                let program = if has_filter { 3u32 } else { 0u32 };
                let color_matrix = n.style.color_filter.unwrap_or([0.0; 20]);
                rn.payload = NodePayload::Mesh { verts: v, uvs: uvc, colors: col, indices: idx, texture: tex_id, program, color_matrix };
            }
            NodeKind::Text { content } => {
                let s = &n.style;
                // 复用 layout 阶段 TextLayout（taffy 选定 max_width 测），不重测：
                // 用 rect.w（stretch 后整数宽）重测，短文本 intrinsic 亚像素超 available
                // 会误判换行。fallback（text_layouts 空，如 test 未走 solve）：用 rect.w 测。
                let mut layout = scene
                    .text_layouts
                    .get(n.id.index())
                    .cloned()
                    .flatten()
                    .unwrap_or_else(|| {
                        measure_text(
                            content, s.font_size, s.line_height, s.letter_spacing,
                            s.text_align, s.white_space_nowrap, Some(rect.w), font,
                        )
                    });
                let off_x = resolve_lp(s.taffy_style.border.left) + resolve_lp(s.taffy_style.padding.left);
                let off_y = resolve_lp(s.taffy_style.border.top) + resolve_lp(s.taffy_style.padding.top);
                if off_x != 0.0 || off_y != 0.0 {
                    bake_content_offset(&mut layout, off_x, off_y);
                }
                rn.payload = NodePayload::Text {
                    layout, font_size: s.font_size,
                    color: anim.and_then(|a| a.text_color).unwrap_or(s.color), program: 1,
                };
            }
        }
        // 算本帧 hash，与上帧比。相等（且有基线）→ payload 改回 Unchanged。
        let h = crate::render::dirty::node_hash(rn);
        new_hashes[pos] = h;
        if baselined && prev_hashes[pos] == h {
            rn.payload = NodePayload::Unchanged;
        }
    }
    let clips = batch::assign_sort_keys(scene, &mut nodes);
    // max_sort 在 reorder/merge 前算（内容 sort_key；scrollbar 用 max+1 排内容后）。
    let max_sort = nodes.iter().map(|n| n.sort_key).max().unwrap_or(0);
    batch::reorder_for_batching(scene, &mut nodes);
    let mut nodes = merge::merge_meshes(nodes);
    // 合成 scrollbar thumb（merge 后追加——sentinel id = container|V/H_THUMB_FLAG 高位，
    // batch.rs reorder 用 node 索引 scene.nodes，sentinel 越界；故不参与 batch，独立 quad 末尾追加）。
    for n in scene.nodes.values() {
        let nid = n.id;
        if let Some(s) = scene.scroll.get(nid) {
            if crate::scroll::effective(n.style.overflow_y, s.content_size.1, s.viewport_size.1) {
                if let Some(r) = crate::scroll::v_thumb_rect(scene, nid) {
                    let thumb_id = nid.0 as u32 | crate::scroll::V_THUMB_FLAG;
                    nodes.push(thumb_render_node(thumb_id, r, max_sort + 1));
                }
            }
            if crate::scroll::effective(n.style.overflow_x, s.content_size.0, s.viewport_size.0) {
                if let Some(r) = crate::scroll::h_thumb_rect(scene, nid) {
                    let thumb_id = nid.0 as u32 | crate::scroll::H_THUMB_FLAG;
                    nodes.push(thumb_render_node(thumb_id, r, max_sort + 1));
                }
            }
        }
    }
    (FrameData { nodes, clips }, new_hashes)
}

/// 按 background-size 算 rect → atlas UV 映射。
///
/// cover/contain 用统一 span 公式，区别在缩放比 s 取 max（cover）/ min（contain）：
/// - Cover：s=max，UV 跨度 ≤ 子区（内收），取子区中央可见部分，图边缘被裁。
/// - Contain：s=min，UV 跨度 ≥ 子区（外扩），rect 边缘采样落到子区外 → clamp 到图边缘
///   透明像素 → 留白透出 background-color。
/// - Stretch：整子区拉伸填满。
fn fit_uv(size: crate::style::resolved::BackgroundSize, rect: &Rect, meta: &TexMeta) -> ([f32; 2], [f32; 2]) {
    use crate::style::resolved::BackgroundSize;
    let (rw, rh) = (rect.w, rect.h);
    let (iw, ih) = (meta.width as f32, meta.height as f32);
    let (uv_min, uv_max) = (meta.uv_min, meta.uv_max);
    if iw <= 0.0 || ih <= 0.0 { return (uv_min, uv_max); }
    let (auw, auh) = (uv_max[0] - uv_min[0], uv_max[1] - uv_min[1]);
    match size {
        // Stretch/Contain：UV 整子区。Contain 由 build_render_nodes 缩 geometry（左上子矩形），
        // UV 配合用整 [0,1]；Stretch 整子区拉伸填满。
        BackgroundSize::Stretch | BackgroundSize::Contain => (uv_min, uv_max),
        BackgroundSize::Cover => {
            // cover 左上 CSS 0% 0%：图顶对齐容器顶、图左对齐容器左，右下溢出裁切。
            // design 顶 ↔ texture 子区顶（v 大，Unity v=1=图顶）；v 取子区顶 [uv_max[1]-v_span, uv_max[1]]。
            // u 取子区左 [uv_min[0], uv_min[0]+u_span]。
            let s = (rw / iw).max(rh / ih);
            let u_span = auw * rw / (iw * s);
            let v_span = auh * rh / (ih * s);
            ([uv_min[0], uv_max[1] - v_span],
             [uv_min[0] + u_span, uv_max[1]])
        }
    }
}

/// 把 taffy `LengthPercentage` 解析为 px。
///
/// - `Length(v)` → v。
/// - `Percent(_)` → 0.0。**已知缺口**（记 ledger）：渲染阶段无父 content-box 宽度上下文，
///   无法解析百分比的 padding/border。`style::mapping::parse_four` 对 padding/border
///   只产 `Length`（裸数字/px），故实际不会命中 Percent 分支；若未来 CSS 允许百分比
///   padding/border，需在 layout 阶段把解析结果写回 ResolvedStyle。
fn resolve_lp(lp: LengthPercentage) -> f32 {
    match lp {
        LengthPercentage::Length(v) => v,
        LengthPercentage::Percent(_) => 0.0,
    }
}

/// 烤 content 偏移进 TextLayout 每个 glyph 的 (x, y)（pen = GO-local）。
/// layout 是刚由 measure_text 产的 owned 值，直接 mutate。
fn bake_content_offset(layout: &mut crate::text::layout::TextLayout, off_x: f32, off_y: f32) {
    for line in &mut layout.lines {
        for run in &mut line.runs {
            for g in &mut run.glyphs {
                g.x += off_x;
                g.y += off_y;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::texture::{TexMeta, TextureRegistry};
    use crate::scene::node::*;
    use crate::style::resolved::{BackgroundSize, BorderRadius, CornerRadius, ResolvedStyle, TextAlign};
    use crate::text::layout::measure_text;
    use taffy::style::{Dimension, LengthPercentage};

    /// 测试字体：仓库内 DejaVuSans.ttf（跨平台一致），缺则跳过。
    fn test_font() -> Option<Font> {
        let p = format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"));
        Font::from_path(&p).ok()
    }

    /// 构造一个带 layout_rect 的 Container Node。
    fn container_node(id: usize, parent: Option<usize>, rect: Rect, bg: Option<[f32; 4]>) -> Node {
        let mut n = Node::default();
        n.id = NodeId(id as u32);
        n.parent = parent.map(|p| NodeId(p as u32));
        n.kind = NodeKind::Container;
        n.layout_rect = rect;
        n.style.background_color = bg;
        n
    }

    #[test]
    fn build_container_produces_mesh_quad() {
        // root 红底 10x10 → Mesh payload，4 verts / 6 indices，背景色烤进 colors。
        let mut scene = Scene::from_nodes(vec![container_node(
            0,
            None,
            Rect {
                x: 1.0,
                y: 2.0,
                w: 10.0,
                h: 10.0,
            },
            Some([1.0, 0.0, 0.0, 1.0]),
        )], vec![]);
        let font = test_font().expect("need test font for build_render_nodes");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        let rns = &frame.nodes;
        assert_eq!(rns.len(), 1);
        match &rns[0].payload {
            NodePayload::Mesh {
                verts,
                indices,
                colors,
                texture,
                program,
                ..
            } => {
                assert_eq!(verts.len(), 4);
                assert_eq!(indices.len(), 6);
                assert_eq!(*texture, 0, "Container 无贴图");
                assert_eq!(*program, 0);
                for c in colors {
                    assert_eq!(*c, [1.0, 0.0, 0.0, 1.0]);
                }
            }
            _ => panic!("expected Mesh payload"),
        }
        // world_matrix 纯平移 → tx/ty = layout_rect x/y
        assert_eq!(rns[0].world_matrix[4], 1.0);
        assert_eq!(rns[0].world_matrix[5], 2.0);
    }

    #[test]
    fn build_image_uses_registered_tex_id() {
        let mut a = Node::default();
        a.kind = NodeKind::Image { src: "logo.png".into() };
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 5.0, h: 5.0 };
        let mut scene = Scene::from_nodes(vec![a], vec![]);

        let font = test_font().expect("need test font");
        let mut tex = TextureRegistry::default();
        // 非平凡 uv（atlas 子区）：TL=(0.25,0) BR=(0.5,1)。
        let tid = { tex.insert("logo.png", TexMeta {
            tex_id: 1, uv_min: [0.25, 0.0], uv_max: [0.5, 1.0], width: 200, height: 100,
        }); 1 };
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { texture, uvs, .. } => {
                assert_eq!(*texture, tid, "注册后 Image.texture == 注册分配的 tex_id");
                assert_ne!(*texture, 0, "已注册 tex_id 不应为 0");
                // UV 按 region 烤 + v 翻转（design y-down 配 Unity y-up）：TL=(umin,vmax)，BR=(umax,vmin)。
                assert_eq!(uvs[0], [0.25, 1.0], "TL == (uv_min.u, uv_max.v)");
                assert_eq!(uvs[2], [0.5, 0.0], "BR == (uv_max.u, uv_min.v)");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_image_unregistered_is_zero() {
        let mut a = Node::default();
        a.kind = NodeKind::Image { src: "logo.png".into() };
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 5.0, h: 5.0 };
        let mut scene = Scene::from_nodes(vec![a], vec![]);

        let font = test_font().expect("need test font");
        let tex = TextureRegistry::default(); // 未注册
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        let got = match &frame.nodes[0].payload {
            NodePayload::Mesh { texture, .. } => *texture,
            _ => panic!("expected Mesh"),
        };
        assert_eq!(got, 0, "未注册 src → tex_id=0 哨兵");
    }

    #[test]
    fn build_image_unregistered_uv_is_full() {
        // 未注册 src → 哨兵 uv (0,0)-(1,1)（与 tex_id=0 白占位配合，UV 无关）。
        let mut a = Node::default();
        a.kind = NodeKind::Image { src: "logo.png".into() };
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 5.0, h: 5.0 };
        let mut scene = Scene::from_nodes(vec![a], vec![]);

        let font = test_font().expect("need test font");
        let tex = TextureRegistry::default(); // 未注册
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { uvs, .. } => {
                assert_eq!(uvs[0], [0.0, 1.0], "未注册 TL == (0,1)（v 翻转）");
                assert_eq!(uvs[2], [1.0, 0.0], "未注册 BR == (1,0)（v 翻转）");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_text_produces_text_layout() {
        let font = match test_font() {
            Some(f) => f,
            None => {
                eprintln!("skip: no test font");
                return;
            }
        };
        let mut n = Node::default();
        n.kind = NodeKind::Text {
            content: "Hello".into(),
        };
        n.style.font_size = 16.0;
        n.style.text_align = TextAlign::Left;
        n.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 20.0,
        };
        let mut scene = Scene::from_nodes(vec![n], vec![]);

        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        let rns = &frame.nodes;
        match &rns[0].payload {
            NodePayload::Text {
                layout,
                font_size,
                program,
                ..
            } => {
                assert_eq!(*font_size, 16.0);
                assert_eq!(*program, 1);
                assert!(!layout.lines.is_empty());
            }
            _ => panic!("expected Text payload"),
        }
    }

    /// pen 必须 GO-local——measure_text 产 content-box 相对坐标，
    /// build_render_nodes 烤 (border_left+padding_left, border_top+padding_top) 偏移。
    /// 设 padding=4px、border=2px → content 偏移 (6, 6)，每 glyph 的 (x,y) 应 +6。
    #[test]
    fn build_text_bakes_content_offset_into_glyph_pen() {
        let font = match test_font() {
            Some(f) => f,
            None => {
                eprintln!("skip: no test font");
                return;
            }
        };
        let mut n = Node::default();
        n.kind = NodeKind::Text {
            content: "AB".into(),
        };
        n.style.font_size = 16.0;
        // padding/border 四向 4px/2px → content 偏移 left=2+4=6, top=2+4=6。
        n.style.taffy_style.padding = taffy::geometry::Rect {
            left: taffy::style::LengthPercentage::Length(4.0),
            right: taffy::style::LengthPercentage::Length(4.0),
            top: taffy::style::LengthPercentage::Length(4.0),
            bottom: taffy::style::LengthPercentage::Length(4.0),
        };
        n.style.taffy_style.border = taffy::geometry::Rect {
            left: taffy::style::LengthPercentage::Length(2.0),
            right: taffy::style::LengthPercentage::Length(2.0),
            top: taffy::style::LengthPercentage::Length(2.0),
            bottom: taffy::style::LengthPercentage::Length(2.0),
        };
        n.layout_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 20.0,
        };
        let mut scene = Scene::from_nodes(vec![n], vec![]);

        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        let rns = &frame.nodes;
        match &rns[0].payload {
            NodePayload::Text { layout, .. } => {
                let g = &layout.lines[0].runs[0].glyphs;
                assert_eq!(g.len(), 2, "AB = 2 glyphs");
                // 每 glyph 的 y 原 = line.y(0)，+content offset(6) = 6.0。
                assert_eq!(g[0].y, 6.0, "pen_y 烤 content 偏移 (border+padding top=6)");
                assert_eq!(g[1].y, 6.0);
                // 首 glyph x 原 = 0（Left align），+6 = 6.0；次 glyph x 应 > 6（advance）。
                assert_eq!(g[0].x, 6.0, "首 glyph pen_x = align 偏移0 + content 偏移6");
                assert!(g[1].x > 6.0, "次 glyph pen_x > 6（含 advance + 偏移）");
                // codepoint 也顺带验。
                assert_eq!(g[0].codepoint, b'A' as u32);
                assert_eq!(g[1].codepoint, b'B' as u32);
            }
            _ => panic!("expected Text payload"),
        }
    }

    #[test]
    fn build_assigns_monotonic_keys() {
        // 用嵌套 clip 链（root > mid > leaf，每层 clip_rect 开新 mask_context）
        // → 3 个不同 DrawState → 不合并 → 保 3 节点。
        // 验 sort_key 单调（batch 已测，这里走端到端确认 build 接通 assign_sort_keys）。
        let root = container_node(0, None, Rect::default(), None);
        let mid = container_node(1, Some(0), Rect::default(), None);
        let leaf = container_node(2, Some(1), Rect::default(), None);
        let mut scene = Scene::from_nodes(vec![root, mid, leaf], vec![(0, 1), (1, 2)]);
        let root_id = scene.roots[0];
        let mid_id = scene.get(root_id).unwrap().children[0];
        let leaf_id = scene.get(mid_id).unwrap().children[0];
        scene.get_mut(root_id).unwrap().clip_rect = Some(Rect::default()); // 开 mask_context=1
        scene.get_mut(mid_id).unwrap().clip_rect = Some(Rect::default()); // 开 mask_context=2
        scene.get_mut(leaf_id).unwrap().clip_rect = Some(Rect::default()); // 开 mask_context=3

        let font = test_font().expect("need test font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        let rns = &frame.nodes;
        // 3 个不同 mask_context → 不合并 → 保 3 节点；sort_key 经 reorder 重赋后仍单调。
        assert_eq!(rns.len(), 3, "3 个不同 mask_context → 不合并");
        assert!(rns[0].sort_key < rns[1].sort_key);
        assert!(rns[1].sort_key < rns[2].sort_key);
    }

    /// 端到端 merge：root(Container, tex_id=0) > [img A, img B]（同 tex_id=1、
    /// 同 mask_context、AABB 不相交）。reorder 让两 Image 相邻，merge 合两 Image 成 1 个 8-vert
    /// merged mesh；root 是 Container(tex_id=0) 不同 DrawState → 不合。
    /// 结果：FrameData 含恰好 1 个 8-vert Mesh payload（两 Image 合并）。
    #[test]
    fn build_merges_adjacent_same_drawstate_meshes() {
        let root = container_node(
            0,
            None,
            Rect { x: 0.0, y: 0.0, w: 300.0, h: 50.0 },
            None,
        );
        let mut a = Node::default();
        a.kind = NodeKind::Image { src: "a.png".into() };
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        let mut b = Node::default();
        b.kind = NodeKind::Image { src: "a.png".into() };
        b.layout_rect = Rect { x: 100.0, y: 0.0, w: 10.0, h: 10.0 };
        let mut scene = Scene::from_nodes(vec![root, a, b], vec![(0, 1), (0, 2)]);

        let font = test_font().expect("need test font");
        let mut tex = TextureRegistry::default();
        tex.insert(
            "a.png",
            TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 10, height: 10 },
        );

        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        // root(Container, tex_id=0) + 1 merged(Image tex_id=1) = 2 节点（原 3）。
        let mesh_count = frame
            .nodes
            .iter()
            .filter(|n| matches!(&n.payload, NodePayload::Mesh { verts, .. } if verts.len() == 8))
            .count();
        assert_eq!(mesh_count, 1, "两同 atlas Image → 1 个 8-vert merged mesh");
        // merged node 的 world_matrix 应为 IDENTITY（merge_batch 把锚矩阵置 identity），
        // 顶点保持绝对 design 坐标。
        let merged = frame
            .nodes
            .iter()
            .find(|n| matches!(&n.payload, NodePayload::Mesh { verts, .. } if verts.len() == 8))
            .expect("merged node 存在");
        assert!(crate::transform::is_identity(&merged.world_matrix));
        assert!((merged.alpha - 1.0).abs() < 1e-6, "merged alpha=1 防 blob 二次烤");
    }

    #[test]
    fn image_uv_flips_v_for_design_y_down() {
        // design y-down + LoomStage scale (sf,-sf,sf) 把 design 顶映到屏幕上；
        // mesh::quad 固定 TL→(umin,vmin)（texture 底）→ Unity 上下颠倒。须 swap v：TL→(umin,vmax)。
        let mut root = Node::default();
        root.kind = NodeKind::Container;
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut img = Node::default();
        img.kind = NodeKind::Image { src: "a.png".into() };
        img.layout_rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        let mut scene = Scene::from_nodes(vec![root, img], vec![(0, 1)]);
        let font = test_font().expect("need test font");
        let mut tex = TextureRegistry::default();
        tex.insert("a.png", TexMeta { tex_id: 1, uv_min: [0.25, 0.25], uv_max: [0.75, 0.75], width: 10, height: 10 });
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        let img_rn = frame.nodes.iter()
            .find(|n| matches!(&n.payload, NodePayload::Mesh { verts, texture, .. } if *texture == 1 && verts.len() == 4))
            .expect("img 4-vert mesh");
        if let NodePayload::Mesh { uvs, .. } = &img_rn.payload {
            // vert 序 TL,TR,BR,BL（mesh::quad）；TL（design 顶左）→ (umin, vmax) = (0.25, 0.75)。
            assert!((uvs[0][0] - 0.25).abs() < 1e-3, "TL u=0.25，got {}", uvs[0][0]);
            assert!((uvs[0][1] - 0.75).abs() < 1e-3, "TL v=0.75（texture 顶，配 design 顶），got {}", uvs[0][1]);
        }
    }

    /// build_render_nodes 读 anim.opacity/bg_color override（replace-override）。
    /// CSS opacity=1.0、bg=红；anim opacity=0.25、bg=蓝 → alpha=0.25、Mesh colors=蓝。
    #[test]
    fn build_reads_anim_opacity_and_bg_override() {
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Some([1.0, 0.0, 0.0, 1.0]));
        n.style.opacity = 1.0;
        let mut scene = Scene::from_nodes(vec![n], vec![]);
        let rid = scene.roots[0];
        // anim override：opacity=0.25、bg=蓝
        scene.anim.ensure(rid.0 as usize + 1);
        scene.anim.0[rid.0 as usize].opacity = Some(0.25);
        scene.anim.0[rid.0 as usize].bg_color = Some([0.0, 0.0, 1.0, 1.0]);

        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        assert!((frame.nodes[0].alpha - 0.25).abs() < 1e-5, "anim.opacity override → alpha=0.25");
        match &frame.nodes[0].payload {
            NodePayload::Mesh { colors, .. } => {
                assert_eq!(*colors.first().unwrap(), [0.0, 0.0, 1.0, 1.0], "anim.bg_color override → 蓝");
            }
            _ => panic!("expected Mesh"),
        }
    }

    // ── 合成 scrollbar thumb ─────────────────────────

    #[test]
    fn effective_scroll_container_emits_thumb_node() {
        use crate::style::resolved::{OverflowMode, ResolvedStyle};

        // 构造：overflow_y=Scroll 容器 + content>viewport → effective
        let mut scroll_style = ResolvedStyle::default();
        scroll_style.overflow_y = OverflowMode::Scroll;
        let entries: Vec<(
            Option<usize>,
            NodeKind,
            ResolvedStyle,
            Vec<String>,
            Option<String>,
            bool,
            Option<i32>,
        )> = vec![
            (None, NodeKind::Container, scroll_style.clone(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        let root_id = scene.roots[0];
        let c0 = scene.get(root_id).unwrap().children[0];
        let c1 = scene.get(root_id).unwrap().children[1];
        scene.get_mut(root_id).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        scene.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        scene.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 }; // content_y=200 > viewport=100
        crate::scroll::refresh_content_sizes(&mut scene);
        crate::scene::transform::compute_world_transforms(&mut scene);

        let font = test_font().expect("need test font");
        let (fd, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        let thumbs: Vec<_> = fd
            .nodes
            .iter()
            .filter(|n| n.node_id & crate::scroll::V_THUMB_FLAG != 0)
            .collect();
        assert!(!thumbs.is_empty(), "垂直 thumb 追加");
        // 验 thumb 是 Mesh quad 半透明灰
        let thumb = thumbs[0];
        assert_eq!(thumb.mask_context, MaskContext(0), "thumb mask_context=0");
        assert!(thumb.sort_key > 0, "thumb sort_key > 0");
        match &thumb.payload {
            NodePayload::Mesh { colors, .. } => {
                assert_eq!(colors[0], [0.6, 0.6, 0.6, 0.6], "半透明灰");
            }
            _ => panic!("thumb 应为 Mesh"),
        }
    }

    #[test]
    fn non_effective_container_no_thumb() {
        // 构造 overflow:auto 但 content < viewport → 非 effective → 无 thumb
        use crate::style::resolved::{OverflowMode, ResolvedStyle};
        let mut scroll_style = ResolvedStyle::default();
        scroll_style.overflow_y = OverflowMode::Auto;
        let entries = vec![
            (None, NodeKind::Container, scroll_style.clone(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        let root_id = scene.roots[0];
        let c0 = scene.get(root_id).unwrap().children[0];
        scene.get_mut(root_id).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        scene.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 }; // content < viewport
        crate::scroll::refresh_content_sizes(&mut scene);
        crate::scene::transform::compute_world_transforms(&mut scene);

        let font = test_font().expect("need test font");
        let (fd, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        let has_thumb = fd
            .nodes
            .iter()
            .any(|n| n.node_id & (crate::scroll::V_THUMB_FLAG | crate::scroll::H_THUMB_FLAG) != 0);
        assert!(!has_thumb, "non-effective 容器无 thumb");
    }

    // ── dirty Unchanged emit ─────────────────────────

    /// 首帧（prev_hashes 空）→ 全 emit Mesh，无 Unchanged。
    #[test]
    fn build_first_frame_all_emit_no_unchanged() {
        let mut scene = Scene::from_nodes(vec![container_node(0, None, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Some([1.0,0.0,0.0,1.0]))], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _hashes) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        // 首帧无 Unchanged（全 Mesh）。
        assert!(frame.nodes.iter().all(|n| !matches!(n.payload, NodePayload::Unchanged)),
            "首帧 prev_hashes 空 → 全 emit");
    }

    /// 第二帧无变化 → 该节点 Unchanged。
    #[test]
    fn build_static_frame_emits_unchanged() {
        let mut scene = Scene::from_nodes(vec![container_node(0, None, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Some([1.0,0.0,0.0,1.0]))], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        // 首帧拿 hash 基线。
        let (_f1, hashes) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        // 第二帧无变化 → Unchanged。
        let (f2, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &hashes);
        // f2 含 1 个 Unchanged（merge 后该节点 passthrough 仍 Unchanged）。
        assert!(f2.nodes.iter().any(|n| matches!(n.payload, NodePayload::Unchanged)),
            "静态帧未变节点 → Unchanged");
    }

    /// 第二帧 style 变（bg color）→ 该节点重 emit Mesh（非 Unchanged）。
    #[test]
    fn build_changed_frame_re_emits() {
        let mut scene = Scene::from_nodes(vec![container_node(0, None, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Some([1.0,0.0,0.0,1.0]))], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (_f1, hashes) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        // 改 bg color。
        let rid = scene.roots[0];
        scene.get_mut(rid).unwrap().style.background_color = Some([0.0,1.0,0.0,1.0]);
        let (f2, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &hashes);
        assert!(f2.nodes.iter().all(|n| !matches!(n.payload, NodePayload::Unchanged)),
            "bg color 变 → 重 emit Mesh（colors[0] hash 不等）");
    }

    /// reload（节点数变，prev_hashes 长度不符）→ 全 emit（无基线）。
    #[test]
    fn build_reload_clears_baseline() {
        let mut scene = Scene::from_nodes(vec![container_node(0, None, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Some([1.0,0.0,0.0,1.0]))], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (_f1, hashes) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        // prev_hashes 长度 > 节点数（模拟 reload 后旧 hash 表残留）→ 无基线 → 全 emit。
        let mut stale = hashes.clone();
        stale.push(999);
        let (f2, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &stale);
        assert!(f2.nodes.iter().all(|n| !matches!(n.payload, NodePayload::Unchanged)),
            "prev_hashes 长度不符 → 全 emit（防错位）");
    }

    /// render 复用 layout 阶段 TextLayout，不重测。
    /// 验证：solve 填 scene.text_layouts，build_render_nodes 的 Text payload 行数
    /// == text_layouts 行数（render 直接读，不再 measure_text）。
    #[test]
    fn render_text_payload_matches_layout_text_layout() {
        let font = match test_font() {
            Some(f) => f,
            None => { eprintln!("skip: no test font"); return; }
        };
        let content = "the layout reuse check text";
        let fs = 16.0;
        let mut root_s = ResolvedStyle::default();
        root_s.taffy_style.size.width = Dimension::Length(120.0);
        let mut text_s = ResolvedStyle::default();
        text_s.font_size = fs;
        let entries = vec![
            (None, NodeKind::Container, root_s, vec![], None, false, None),
            (Some(0), NodeKind::Text { content: content.into() }, text_s, vec![], None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        let tex = TextureRegistry::default();
        crate::layout::solve(&mut scene, &font, (120.0, 100.0), &tex);
        let text_id = scene.get(scene.roots[0]).unwrap().children[0];
        assert!(scene.text_layouts[text_id.index()].is_some(), "solve 应为 Text 节点填 text_layouts");
        let layout_lines = scene.text_layouts[text_id.index()].as_ref().unwrap().lines.len();
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        let render_lines = match &frame.nodes[1].payload {
            NodePayload::Text { layout, .. } => layout.lines.len(),
            _ => panic!("expected Text payload"),
        };
        assert_eq!(render_lines, layout_lines, "render 应复用 layout TextLayout（行数一致，不重测）");
    }

    /// 长文本回归（intrinsic 远超 container）仍正确换行。
    #[test]
    fn render_long_text_still_wraps_with_layout_reuse() {
        let font = match test_font() {
            Some(f) => f,
            None => { eprintln!("skip: no test font"); return; }
        };
        let content = "The quick brown fox jumps over the lazy dog again and again";
        let fs = 16.0;
        let intrinsic = measure_text(content, fs, 0.0, 0.0, TextAlign::Left, false, None, &font).text_width;
        let container_w = 100.0;
        assert!(intrinsic > container_w, "测试前置：长文本 intrinsic 应远超 container");
        let mut root_s = ResolvedStyle::default();
        root_s.taffy_style.size.width = Dimension::Length(container_w);
        let mut text_s = ResolvedStyle::default();
        text_s.font_size = fs;
        let entries = vec![
            (None, NodeKind::Container, root_s, vec![], None, false, None),
            (Some(0), NodeKind::Text { content: content.into() }, text_s, vec![], None, false, None),
        ];
        let mut scene = Scene::build(&entries);
        let tex = TextureRegistry::default();
        crate::layout::solve(&mut scene, &font, (container_w, 100.0), &tex);
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        let lines = match &frame.nodes[1].payload {
            NodePayload::Text { layout, .. } => layout.lines.len(),
            _ => panic!("expected Text payload"),
        };
        assert!(lines >= 2, "长文本 intrinsic={:.1} container={} 应换行，got {} 行", intrinsic, container_w, lines);
    }

    // ── fit_uv ────────────────────────────────────────

    #[test]
    fn fit_uv_stretch_returns_full_subregion() {
        let meta = TexMeta { tex_id: 1, uv_min: [0.25, 0.25], uv_max: [0.75, 0.75], width: 100, height: 100 };
        let rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 50.0 };
        let (a, b) = fit_uv(BackgroundSize::Stretch, &rect, &meta);
        assert_eq!(a, [0.25, 0.25]);
        assert_eq!(b, [0.75, 0.75], "Stretch 整子区不变");
    }

    #[test]
    fn fit_uv_cover_insets_to_top_left() {
        // 正方图 100×100，长方容器 200×50：scale=max(200/100,50/100)=2，
        // 图左上对齐容器（CSS position 0% 0%），右下溢出裁切 → UV 从 uv_min 起。
        let meta = TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 100, height: 100 };
        let rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 50.0 };
        let (a, b) = fit_uv(BackgroundSize::Cover, &rect, &meta);
        // u_span = 1.0（宽填满），v_span = 0.25（高内收）；v 取子区顶 0.75..1.0（图顶对齐容器顶）。
        assert!((a[0] - 0.0).abs() < 1e-5, "Cover u_min=0");
        assert!((b[0] - 1.0).abs() < 1e-5, "Cover u_max=1（宽填满）");
        assert!((a[1] - 0.75).abs() < 1e-5, "Cover v_min=0.75（子区顶起）");
        assert!((b[1] - 1.0).abs() < 1e-5, "Cover v_max=1.0（子区顶，图顶对齐容器顶）");
    }

    #[test]
    fn fit_uv_contain_returns_full_subregion() {
        // contain 由 build_render_nodes 缩 geometry（左上子矩形），fit_uv 只返整子区 UV。
        // 100×100 图，100×200 容器：geometry 缩到 100×100（左上），UV 整 [0,1]。
        let meta = TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 100, height: 100 };
        let rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 200.0 };
        let (a, b) = fit_uv(BackgroundSize::Contain, &rect, &meta);
        assert_eq!(a, [0.0, 0.0], "Contain u_min/v_min=0（整子区）");
        assert_eq!(b, [1.0, 1.0], "Contain u_max/v_max=1（整子区，geometry 缩在 build_render_nodes）");
    }

    #[test]
    fn fit_uv_zero_image_size_returns_full() {
        let meta = TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 0, height: 0 };
        let rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let (a, b) = fit_uv(BackgroundSize::Cover, &rect, &meta);
        assert_eq!((a, b), ([0.0, 0.0], [1.0, 1.0]), "退化：图 0px → 整子区");
    }

    // ── Container bg-image ────────────────────────────

    #[test]
    fn build_container_with_bg_image_uses_tex_id_and_fit_uv() {
        // Container 设 background-image + background-size:cover → Mesh texture=tex_id(非0)
        // + UV 按 cover 内收 + v 翻转。
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 200.0, h: 50.0 }, None);
        n.style.background_image = Some("a.png".into());
        n.style.background_size = BackgroundSize::Cover;
        let mut scene = Scene::from_nodes(vec![n], vec![]);

        let font = test_font().expect("need font");
        let mut tex = TextureRegistry::default();
        tex.insert("a.png", TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 100, height: 100 });
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { texture, uvs, colors, .. } => {
                assert_eq!(*texture, 1, "带图 Container texture=tex_id(1)");
                // cover v 取子区顶 0.75..1.0（图顶对齐容器顶），v 翻转后 TL=(0, 1.0)
                assert!((uvs[0][0] - 0.0).abs() < 1e-5, "TL u=0");
                assert!((uvs[0][1] - 1.0).abs() < 1e-5, "TL v=1.0（cover 图顶对齐容器顶 + v 翻转）");
                // 无 background-color → 顶点色透明（图独立显示）
                assert_eq!(*colors.first().unwrap(), [0.0, 0.0, 0.0, 0.0], "无底色 → 透明顶点色");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_container_bg_image_contain_shrinks_geometry() {
        // contain：图完整放入，geometry 缩到子矩形（左上 CSS position 0% 0%），右下留白。
        // 100×100 图，200×100 容器：s=min(2,1)=1，子矩形 100×100 左上 → verts xmax=100（右留白 100）。
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 200.0, h: 100.0 }, None);
        n.style.background_image = Some("a.png".into());
        n.style.background_size = BackgroundSize::Contain;
        let mut scene = Scene::from_nodes(vec![n], vec![]);

        let font = test_font().expect("need font");
        let mut tex = TextureRegistry::default();
        tex.insert("a.png", TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 100, height: 100 });
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        if let NodePayload::Mesh { verts, .. } = &frame.nodes[0].payload {
            let xmax = verts.iter().map(|v| v[0]).fold(f32::MIN, f32::max);
            assert!((xmax - 100.0).abs() < 1e-2, "contain 子矩形 xmax=100（图缩放宽，右留白），got {}", xmax);
        } else { panic!("expected Mesh"); }
    }

    #[test]
    fn build_container_bg_image_coexists_with_bg_color() {
        // background-color + background-image 共存：顶点色=底色 tint + texture=tex_id
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, Some([0.0, 1.0, 0.0, 1.0]));
        n.style.background_image = Some("a.png".into());
        n.style.background_size = BackgroundSize::Stretch;
        let mut scene = Scene::from_nodes(vec![n], vec![]);

        let font = test_font().expect("need font");
        let mut tex = TextureRegistry::default();
        tex.insert("a.png", TexMeta { tex_id: 2, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 50, height: 50 });
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { texture, colors, uvs, .. } => {
                assert_eq!(*texture, 2, "texture=tex_id(2)");
                assert_eq!(*colors.first().unwrap(), [0.0, 1.0, 0.0, 1.0], "顶点色=绿底（tint）");
                // Stretch 整子区 + v 翻转：TL=(0,1)
                assert_eq!(uvs[0], [0.0, 1.0], "Stretch TL=(0,1)（v 翻转）");
            }
            _ => panic!("expected Mesh"),
        }
    }

    // ── program 号（坑 79 bg-image 合成）──────────────

    #[test]
    fn build_container_bg_image_hit_sets_program_2() {
        // Container 设 background-image 且纹理命中 → program=2（CSS 合成）。
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, Some([0.0, 1.0, 0.0, 1.0]));
        n.style.background_image = Some("a.png".into());
        let mut scene = Scene::from_nodes(vec![n], vec![]);
        let font = test_font().expect("need font");
        let mut tex = TextureRegistry::default();
        tex.insert("a.png", TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 10, height: 10 });
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { program, texture, .. } => {
                assert_ne!(*texture, 0, "命中纹理 tex_id≠0");
                assert_eq!(*program, 2, "Container+bg-image 命中 → program=2");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_container_without_bg_image_keeps_program_0() {
        // Container 无 bg-image → program=0（tex*vcol，白占位×bg-color=bg-color）。
        let n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, Some([1.0, 0.0, 0.0, 1.0]));
        let mut scene = Scene::from_nodes(vec![n], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { program, .. } => {
                assert_eq!(*program, 0, "无 bg-image → program=0");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_container_bg_image_unregistered_keeps_program_0() {
        // Container 设 bg-image 但纹理未注册（哨兵 tex_id=0）→ program=0（不走合成，白占位）。
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, Some([1.0, 0.0, 0.0, 1.0]));
        n.style.background_image = Some("missing.png".into());
        let mut scene = Scene::from_nodes(vec![n], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { program, texture, .. } => {
                assert_eq!(*texture, 0, "未注册 → tex_id=0 哨兵");
                assert_eq!(*program, 0, "未注册 → program=0（不走合成）");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_image_node_keeps_program_0() {
        // Image 节点 program=0（tex*vcol，图透明区透下层）——零改回归。
        let mut root = Node::default();
        root.kind = NodeKind::Container;
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut img = Node::default();
        img.kind = NodeKind::Image { src: "a.png".into() };
        img.layout_rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        let mut scene = Scene::from_nodes(vec![root, img], vec![(0, 1)]);
        let font = test_font().expect("need font");
        let mut tex = TextureRegistry::default();
        tex.insert("a.png", TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 10, height: 10 });
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        let img_rn = frame.nodes.iter()
            .find(|n| matches!(&n.payload, NodePayload::Mesh { texture, .. } if *texture == 1))
            .expect("img mesh");
        if let NodePayload::Mesh { program, .. } = &img_rn.payload {
            assert_eq!(*program, 0, "Image → program=0（零改）");
        }
    }

    // ── v1.3 color_filter → program=3 + nine_slice 分流 ──────────

    #[test]
    fn build_container_with_filter_sets_program_3() {
        // Container + filter:grayscale(1) → program=3 + color_matrix 灰化矩阵
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 }, Some([1.0, 0.0, 0.0, 1.0]));
        n.style.color_filter = Some(crate::style::color_filter::grayscale());
        let mut scene = Scene::from_nodes(vec![n], vec![]);
        crate::scene::transform::compute_world_transforms(&mut scene);
        let font = test_font().expect("need font");
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { program, color_matrix, .. } => {
                assert_eq!(*program, 3, "filter → program=3");
                assert!((color_matrix[0] - 0.299).abs() < 1e-4, "color_matrix 含灰化矩阵");
            }
            _ => panic!("expected Mesh"),
        }
    }

    /// Container + bg-image(命中) + filter → program=4（BG_COMPOSITE+COLOR_FILTER 双 keyword，spec §3.2）。
    /// I1 回归：split program=3 → 3（filter 无 bg-image）/ 4（filter+bg-image 双 keyword）。
    /// program=4 由 MaterialManager.cs 同时 EnableKeyword COLOR_FILTER + BG_COMPOSITE，
    /// 让 shader 走 `tex.rgb*tex.a + vcol.rgb*(1-tex.a)`（CSS 合成）后再跑 COLOR_FILTER 后处理。
    #[test]
    fn build_container_with_bg_image_and_filter_sets_program_4() {
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 }, Some([1.0, 0.0, 0.0, 1.0]));
        n.style.background_image = Some("a.png".into());
        n.style.background_size = BackgroundSize::Stretch;
        n.style.color_filter = Some(crate::style::color_filter::grayscale());
        let mut scene = Scene::from_nodes(vec![n], vec![]);

        let font = test_font().expect("need font");
        let mut tex = TextureRegistry::default();
        tex.insert("a.png", TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 10, height: 10 });
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { program, texture, color_matrix, .. } => {
                assert_eq!(*texture, 1, "bg-image 命中 → texture≠0");
                assert_eq!(*program, 4, "bg-image+filter → program=4（BG_COMPOSITE+COLOR_FILTER 双 keyword，spec §3.2）");
                assert!((color_matrix[0] - 0.299).abs() < 1e-4, "color_matrix 含灰化矩阵");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_container_with_slice_uses_nine_slice() {
        // Container + bg-image + border-image-slice → nine_slice mesh（16 顶点）
        let mut tex = TextureRegistry::default();
        tex.insert("skin.png", TexMeta { tex_id: 1, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 48, height: 48 });
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 }, Some([1.0, 0.0, 0.0, 1.0]));
        n.style.background_image = Some("skin.png".into());
        n.style.background_size = BackgroundSize::Stretch;
        n.style.border_image_slice = Some(crate::style::resolved::SliceInsets { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 });
        let mut scene = Scene::from_nodes(vec![n], vec![]);
        crate::scene::transform::compute_world_transforms(&mut scene);
        let font = test_font().expect("need font");
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { verts, .. } => {
                assert_eq!(verts.len(), 16, "slice → nine_slice 16 顶点");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_container_no_filter_keeps_program_0_or_2() {
        // 零回归：无 filter → program 0（无图）/ 2（bg-image 命中）
        let mut scene = Scene::from_nodes(vec![container_node(0, None, Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 }, Some([1.0, 0.0, 0.0, 1.0]))], vec![]);
        crate::scene::transform::compute_world_transforms(&mut scene);
        let font = test_font().expect("need font");
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        if let NodePayload::Mesh { program, .. } = &frame.nodes[0].payload {
            assert_eq!(*program, 0, "无图无 filter → program=0");
        }
    }

    #[test]
    fn build_container_bg_image_unregistered_falls_back_texture_zero() {
        // url 未注册 → texture=0（白占位退化），不 panic
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, None);
        n.style.background_image = Some("missing.png".into());
        let mut scene = Scene::from_nodes(vec![n], vec![]);

        let font = test_font().expect("need font");
        let tex = TextureRegistry::default(); // 未注册
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { texture, .. } => {
                assert_eq!(*texture, 0, "未注册 url → texture=0 哨兵");
            }
            _ => panic!("expected Mesh"),
        }
    }

    #[test]
    fn build_container_no_bg_image_keeps_texture_zero() {
        // 无 background-image → 现状 texture:0（零回归）
        let mut scene = Scene::from_nodes(vec![container_node(0, None, Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, Some([1.0, 0.0, 0.0, 1.0]))], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { texture, .. } => assert_eq!(*texture, 0, "无图 Container texture=0"),
            _ => panic!("expected Mesh"),
        }
    }

    // ── border-radius Tests (T5) ────────────────────────────

    #[test]
    fn container_zero_radius_uses_quad() {
        // 未设 border-radius（默认全 0）→ 走 quad（4 顶点）
        let mut scene = Scene::from_nodes(vec![container_node(0, None, Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 }, Some([1.0, 0.0, 0.0, 1.0]))], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        let rn = &frame.nodes[0];
        match &rn.payload {
            NodePayload::Mesh { verts, .. } => {
                assert_eq!(verts.len(), 4, "radius=0 走 quad（4 顶点），得 {}", verts.len());
            }
            other => panic!("期望 Mesh，得 {:?}", other),
        }
    }

    #[test]
    fn container_radius_uses_rounded_rect() {
        // border-radius:8px → 走 rounded_rect（顶点 >4）
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 }, Some([1.0, 0.0, 0.0, 1.0]));
        n.style.border_radius = BorderRadius {
            corners: [CornerRadius {
                h: LengthPercentage::Length(8.0),
                v: LengthPercentage::Length(8.0),
            }; 4],
        };
        let mut scene = Scene::from_nodes(vec![n], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        let rn = &frame.nodes[0];
        match &rn.payload {
            NodePayload::Mesh { verts, .. } => {
                assert!(verts.len() > 4, "radius>0 走 rounded_rect（顶点>4），得 {}", verts.len());
            }
            other => panic!("期望 Mesh，得 {:?}", other),
        }
    }

    #[test]
    fn container_radius_percent_resolved() {
        // border-radius:50% × 80×80 rect → resolve 成 40 → rounded_rect（顶点>4）
        // 使用 container_node 直接设 layout_rect，无需 solve。
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 }, Some([1.0, 0.0, 0.0, 1.0]));
        n.style.border_radius = BorderRadius {
            corners: [CornerRadius {
                h: LengthPercentage::Percent(0.5),
                v: LengthPercentage::Percent(0.5),
            }; 4],
        };
        let mut scene = Scene::from_nodes(vec![n], vec![]);
        let font = test_font().expect("need font");
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &TextureRegistry::default(), &[]);
        let rn = &frame.nodes[0];
        match &rn.payload {
            NodePayload::Mesh { verts, .. } => {
                assert!(verts.len() > 4, "% resolve 后 radius>0 → rounded_rect，得 {}", verts.len());
            }
            other => panic!("期望 Mesh，得 {:?}", other),
        }
    }

    #[test]
    fn container_bg_image_with_radius_uses_rounded_rect() {
        // bg-image + border-radius 共存：texture 非零 AND 走 rounded_rect（verts>4）
        let mut n = container_node(0, None, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, Some([0.0, 1.0, 0.0, 1.0]));
        n.style.background_image = Some("a.png".into());
        n.style.background_size = BackgroundSize::Stretch;
        n.style.border_radius = BorderRadius {
            corners: [CornerRadius {
                h: LengthPercentage::Length(12.0),
                v: LengthPercentage::Length(12.0),
            }; 4],
        };
        let mut scene = Scene::from_nodes(vec![n], vec![]);

        let font = test_font().expect("need font");
        let mut tex = TextureRegistry::default();
        tex.insert("a.png", TexMeta { tex_id: 2, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: 50, height: 50 });
        crate::scene::transform::compute_world_transforms(&mut scene);
        let (frame, _) = build_render_nodes(&scene, &font, &tex, &[]);
        match &frame.nodes[0].payload {
            NodePayload::Mesh { texture, verts, .. } => {
                assert_eq!(*texture, 2, "bg-image+radius: texture 非零");
                assert!(verts.len() > 4, "bg-image+radius: rounded_rect（顶点>4），得 {}", verts.len());
            }
            _ => panic!("expected Mesh"),
        }
    }
}
