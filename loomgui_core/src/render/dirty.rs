//! dirty hash：逐节点算 u64 hash，供 Stage 跨帧比较决定 emit Unchanged 还是重传。
//! 字段集对照 blob 公共头列 + payload 摘要。碰撞最坏 1 帧延迟，不破正确性。
//! 注：hash 碰撞最坏 1 帧视觉延迟；若 profiling 显示遗漏可换全量 hash。

use crate::render::node::{RenderNode, NodePayload};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// 算节点 dirty hash。调用方保证 payload 为 Mesh/Text（非 Unchanged）。
/// Unchanged 防御性返回 0（不该被调）。
pub fn node_hash(rn: &RenderNode) -> u64 {
    let mut h = DefaultHasher::new();
    // 公共头列（与 blob 公共头字段一致）。
    for &v in rn.world_matrix.iter() { v.to_le_bytes().hash(&mut h); } // Affine2=[f32;6]，含 scroll_pos
    rn.visible.hash(&mut h);
    rn.alpha.to_le_bytes().hash(&mut h);
    rn.grayed.hash(&mut h);
    for &v in rn.color_tint.iter() { v.to_le_bytes().hash(&mut h); }  // [f32;4]
    (match rn.blend { crate::render::node::BlendMode::Normal => 0u8 }).hash(&mut h);
    // sort_key/mask_context **不进 hash**：build_render_nodes 在 assign_sort_keys 之前调
    // node_hash，此刻两者仍是占位值（0/MaskContext(0)），hash 了也只是常量、无贡献。
    // 两者由 scene 结构（DFS 序 + clip 链）决定，结构变必伴随节点增删（prev_hashes
    // 长度变 → baselined=false 全 dirty）或 world/payload 变（hash 仍捕获），无需单独 hash。
    // rn.mask_context.0.hash(&mut h);  // 移除（占位值）
    // rn.sort_key.hash(&mut h);        // 移除（占位值）
    // payload 摘要。
    match &rn.payload {
        NodePayload::Unchanged => 0u64,      // 防御；调用方不应对 Unchanged 调
        NodePayload::Mesh { texture, verts, colors, uvs, .. } => {
            texture.hash(&mut h);
            verts.len().hash(&mut h);
            // colors[0] 在 verts/uvs 之前哈希——恢复 quad 路径原始流顺序
            // （texture→len→colors→verts→uvs），保 quad 零回归 hash 不变量。
            if let Some(c0) = colors.first() { for &v in c0.iter() { v.to_le_bytes().hash(&mut h); } }
            if verts.len() > 4 {
                // rounded_rect mesh：center verts[0] 不随半径变，[2] 只反映 TL 角
                // → 仅 BL/BR/TR 半径变时采样 [0]/[2] 漏掉 → 哈希全量顶点/UV。
                for v in verts.iter() {
                    v[0].to_le_bytes().hash(&mut h);
                    v[1].to_le_bytes().hash(&mut h);
                }
                for uv in uvs.iter() {
                    uv[0].to_le_bytes().hash(&mut h);
                    uv[1].to_le_bytes().hash(&mut h);
                }
            } else {
                // quad (4 verts): O(1) 采样首末顶点 verts[0](TL) + verts[2](BR)
                // quad 尺寸/位置变时 verts.len 仍 4、colors/world 不变，需捕坐标变。
                if let Some(v0) = verts.first() { v0[0].to_le_bytes().hash(&mut h); v0[1].to_le_bytes().hash(&mut h); }
                if let Some(v2) = verts.get(2) { v2[0].to_le_bytes().hash(&mut h); v2[1].to_le_bytes().hash(&mut h); }
                // UV 摘要：background-size 变（cover/contain/stretch 切换，同纹理）→ fit_uv 重算 UV
                // 但 texture/verts/colors 不变 → 须捕 UV 变否则 stale Unchanged。
                if let Some(uv0) = uvs.first() { uv0[0].to_le_bytes().hash(&mut h); uv0[1].to_le_bytes().hash(&mut h); }
                if let Some(uv2) = uvs.get(2) { uv2[0].to_le_bytes().hash(&mut h); uv2[1].to_le_bytes().hash(&mut h); }
            }
            h.finish()
        }
        NodePayload::Text { layout, font_size, color, .. } => {
            font_size.to_le_bytes().hash(&mut h);
            for &v in color.iter() { v.to_le_bytes().hash(&mut h); }
            let glyph_count: usize = layout.lines.iter()
                .map(|l| l.runs.iter().map(|r| r.glyphs.len()).sum::<usize>())
                .sum();
            glyph_count.hash(&mut h);
            // 首 glyph codepoint（捕获内容变）+ pen_x/pen_y（捕获布局变：align 改/constraint 宽改/换行位置改）。
            // 只 hash codepoint 时，text-align Left→Center 或 content offset 烤进时 pen_x/pen_y 变但
            // codepoint 不变 → hash 漏 → 文本位置错。复用 first glyph 引用避免二次遍历。
            match layout.lines.first()
                .and_then(|l| l.runs.first())
                .and_then(|r| r.glyphs.first())
            {
                Some(g) => {
                    g.codepoint.hash(&mut h);
                    g.x.to_le_bytes().hash(&mut h);
                    g.y.to_le_bytes().hash(&mut h);
                }
                None => {
                    0u32.hash(&mut h);  // codepoint sentinel（与旧 unwrap_or(0) 行为一致）
                }
            }
            h.finish()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::mesh::rounded_rect;
    use crate::render::node::{BlendMode, MaskContext, NodePayload, RenderNode};
    use crate::scene::node::Rect;
    use crate::text::layout::{Glyph, GlyphRun, Line, TextLayout};
    use crate::transform::IDENTITY;

    fn mesh_rn(tex: u32, alpha: f32, color0: [f32;4]) -> RenderNode {
        RenderNode {
            node_id: 0, parent_id: None, visible: true, alpha,
            grayed: false, color_tint: [1.0;4],
            world_matrix: IDENTITY, blend: BlendMode::Normal,
            mask_context: MaskContext(0), sort_key: 0,
            payload: NodePayload::Mesh {
                verts: vec![[0.0,0.0];4], uvs: vec![[0.0,0.0];4],
                colors: vec![color0;4], indices: vec![0,1,2,0,2,3],
                texture: tex, program: 0,
            },
        }
    }

    #[test]
    fn identical_nodes_same_hash() {
        let a = mesh_rn(1, 1.0, [1.0,0.0,0.0,1.0]);
        let b = mesh_rn(1, 1.0, [1.0,0.0,0.0,1.0]);
        assert_eq!(node_hash(&a), node_hash(&b), "全等节点 hash 相等");
    }

    #[test]
    fn texture_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0;4]);
        let b = mesh_rn(2, 1.0, [1.0;4]);
        assert_ne!(node_hash(&a), node_hash(&b), "texture 变 → hash 变");
    }

    #[test]
    fn color_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0,0.0,0.0,1.0]);
        let b = mesh_rn(1, 1.0, [0.0,1.0,0.0,1.0]);
        assert_ne!(node_hash(&a), node_hash(&b), "colors[0] 变 → hash 变");
    }

    #[test]
    fn alpha_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0;4]);
        let b = mesh_rn(1, 0.5, [1.0;4]);
        assert_ne!(node_hash(&a), node_hash(&b), "alpha 变 → hash 变");
    }

    #[test]
    fn world_matrix_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0;4]);
        let mut b = mesh_rn(1, 1.0, [1.0;4]);
        b.world_matrix = [1.0, 0.0, 0.0, 1.0, 5.0, 0.0]; // tx=5（scroll 平移）
        assert_ne!(node_hash(&a), node_hash(&b), "world_matrix 变（含 scroll_pos）→ hash 变");
    }

    #[test]
    fn verts_len_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0;4]); // 4 verts
        let mut b = mesh_rn(1, 1.0, [1.0;4]);
        if let NodePayload::Mesh { verts, .. } = &mut b.payload {
            verts.push([9.0, 9.0]); // 5 verts（尺寸变）
        }
        assert_ne!(node_hash(&a), node_hash(&b), "verts.len 变 → hash 变");
    }

    #[test]
    fn unchanged_returns_zero() {
        let mut rn = mesh_rn(1, 1.0, [1.0;4]);
        rn.payload = NodePayload::Unchanged;
        assert_eq!(node_hash(&rn), 0, "Unchanged 防御性返回 0");
    }

    // -----------------------------------------------------------------------
    // Mesh quad 尺寸变（verts.len=4 不变、color/texture/world 不变）
    // → 只看 verts.len 会漏掉坐标变。真实路径：.btn:hover{width:200px} 改 layout_rect.w
    // → Mesh verts[0]/verts[2] 坐标变但 hash 不变 → hover 展开不生效。
    // -----------------------------------------------------------------------
    #[test]
    fn mesh_quad_size_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0,0.0,0.0,1.0]); // 4 verts 全 [0,0]（默认）
        let mut b = mesh_rn(1, 1.0, [1.0,0.0,0.0,1.0]); // 同色/texture/alpha/world
        if let NodePayload::Mesh { verts, .. } = &mut b.payload {
            // 改 quad 尺寸：TL=[0,0] BR=[100,100]（verts.len 仍 4，colors 不变，world 不变）。
            verts[2] = [100.0, 100.0];
        }
        assert_ne!(node_hash(&a), node_hash(&b), "Mesh quad 尺寸变（verts[2] 坐标）→ hash 变");
    }

    // 补：quad 位置变（TL 移动，尺寸不变）也应捕到。
    #[test]
    fn mesh_quad_position_change_changes_hash() {
        let a = mesh_rn(1, 1.0, [1.0;4]);
        let mut b = mesh_rn(1, 1.0, [1.0;4]);
        if let NodePayload::Mesh { verts, .. } = &mut b.payload {
            // 整体平移：verts[0] 移到 [50,50]（尺寸不变、color/world 不变）。
            verts[0] = [50.0, 50.0];
        }
        assert_ne!(node_hash(&a), node_hash(&b), "Mesh quad 位置变（verts[0] 坐标）→ hash 变");
    }

    // -----------------------------------------------------------------------
    // UV 变（background-size 切换：cover/contain/stretch，同纹理）
    // → fit_uv 重算 UV 但 texture/verts/colors 不变 → 须捕 UV 否则 stale Unchanged
    // (:hover 切 background-size 不生效）。
    // -----------------------------------------------------------------------
    #[test]
    fn mesh_uv_change_changes_hash() {
        // background-size 变（同纹理）→ fit_uv 重算 UV，但 texture/verts/colors 不变。
        // 须捕 UV 变否则 stale Unchanged（:hover 切 background-size 不生效）。
        let a = mesh_rn(1, 1.0, [1.0;4]); // uvs 全 [0,0]（默认）
        let mut b = mesh_rn(1, 1.0, [1.0;4]); // 同 texture/verts/colors
        if let NodePayload::Mesh { uvs, .. } = &mut b.payload {
            uvs[0] = [0.25, 0.75]; // 模拟 cover fit_uv 重算的 TL UV
        }
        assert_ne!(node_hash(&a), node_hash(&b), "UV 变（background-size 切换）→ hash 变");
    }

    // -----------------------------------------------------------------------
    // Text payload hash 变化测试。
    // 直接构造 TextLayout（不走 measure_text），排除字体 IO 依赖。
    // -----------------------------------------------------------------------

    fn text_rn(font_size: f32, color: [f32; 4], codepoint: u32, glyph_count: usize) -> RenderNode {
        let glyphs: Vec<Glyph> = (0..glyph_count)
            .map(|i| Glyph {
                glyph_id: 1,
                codepoint: codepoint + i as u32,
                x: i as f32 * 10.0,
                y: 0.0,
                bearing_x: 0.0,
                bearing_y: 0.0,
            })
            .collect();
        let layout = TextLayout {
            text_width: 100.0,
            text_height: 20.0,
            lines: vec![Line {
                y: 0.0,
                height: 20.0,
                baseline: 16.0,
                width: 100.0,
                runs: vec![GlyphRun { font_size, glyphs }],
            }],
        };
        RenderNode {
            node_id: 0,
            parent_id: None,
            visible: true,
            alpha: 1.0,
            grayed: false,
            color_tint: [1.0; 4],
            world_matrix: IDENTITY,
            blend: BlendMode::Normal,
            mask_context: MaskContext(0),
            sort_key: 0,
            payload: NodePayload::Text {
                layout,
                font_size,
                color,
                program: 1,
            },
        }
    }

    #[test]
    fn text_font_size_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(20.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        assert_ne!(node_hash(&a), node_hash(&b), "font_size 变 → hash 变");
    }

    #[test]
    fn text_codepoint_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 66, 3);
        assert_ne!(node_hash(&a), node_hash(&b), "首字 codepoint 变 → hash 变");
    }

    #[test]
    fn text_color_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(16.0, [0.0, 1.0, 0.0, 1.0], 65, 3);
        assert_ne!(node_hash(&a), node_hash(&b), "color 变 → hash 变");
    }

    #[test]
    fn text_glyph_count_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 5);
        assert_ne!(node_hash(&a), node_hash(&b), "glyph_count 变 → hash 变");
    }

    #[test]
    fn text_identical_same_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        let b = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        assert_eq!(node_hash(&a), node_hash(&b), "全等 Text hash 相等");
    }

    // -----------------------------------------------------------------------
    // Text 布局变（text-align Left→Center 或 constraint 宽变）
    // → 首字 pen_x 变但 codepoint/glyph_count/font_size 不变 → hash 漏 → 文本位置错。
    // -----------------------------------------------------------------------
    #[test]
    fn text_first_glyph_pen_x_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3); // 首 glyph x=0
        let mut b = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3); // 同 codepoint/count/size/color
        if let NodePayload::Text { layout, .. } = &mut b.payload {
            // 模拟 align 变：首 glyph pen_x 从 0 → 40（Center 居中偏移）。
            layout.lines[0].runs[0].glyphs[0].x = 40.0;
        }
        assert_ne!(node_hash(&a), node_hash(&b), "首字 pen_x 变（布局变）→ hash 变");
    }

    #[test]
    fn text_first_glyph_pen_y_change_changes_hash() {
        let a = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3); // 首 glyph y=0
        let mut b = text_rn(16.0, [1.0, 0.0, 0.0, 1.0], 65, 3);
        if let NodePayload::Text { layout, .. } = &mut b.payload {
            // 模拟 line baseline 变：首 glyph pen_y 从 0 → 6（content offset 烤进）。
            layout.lines[0].runs[0].glyphs[0].y = 6.0;
        }
        assert_ne!(node_hash(&a), node_hash(&b), "首字 pen_y 变（布局变）→ hash 变");
    }

    // -----------------------------------------------------------------------
    // rounded_rect radius 变 → hash 变（I1 回归测试）
    // rounded_rect 中心 verts[0] 不随半径移动，采样 [0]/[2] 会漏掉 BL/BR/TR 角半径变。
    // -----------------------------------------------------------------------

    fn rounded_rect_rn(radii: &[(f32, f32); 4]) -> RenderNode {
        let rect = Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 };
        let (verts, uvs, colors, indices) = rounded_rect(&rect, [1.0; 4], radii, [0.0, 0.0], [1.0, 1.0]);
        RenderNode {
            node_id: 0, parent_id: None, visible: true, alpha: 1.0,
            grayed: false, color_tint: [1.0; 4],
            world_matrix: IDENTITY, blend: BlendMode::Normal,
            mask_context: MaskContext(0), sort_key: 0,
            payload: NodePayload::Mesh {
                verts, uvs, colors, indices,
                texture: 1, program: 0,
            },
        }
    }

    #[test]
    fn radius_0_to_8_hash_changes() {
        let a = rounded_rect_rn(&[(0.0, 0.0); 4]); // radius 0 → quad fallback (4 verts)
        let b = rounded_rect_rn(&[(8.0, 8.0); 4]); // radius 8 → rounded_rect (~25 verts)
        assert_ne!(node_hash(&a), node_hash(&b), "r=0 → r=8 hash 变");
    }

    #[test]
    fn bl_radius_4_to_5_hash_changes() {
        // 仅 BL 角半径 4→5，同 rect/color/uv/texture，同 verts.len（分段数不变）。
        let a = rounded_rect_rn(&[(0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (4.0, 4.0)]);
        let b = rounded_rect_rn(&[(0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (5.0, 5.0)]);
        assert_ne!(node_hash(&a), node_hash(&b), "仅 BL r=4→5 hash 变");
    }
}
