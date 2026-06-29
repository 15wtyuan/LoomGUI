//! MeshFactory：quad 几何生成。
//!
//! 仅 quad（背景色块 / 图片占位）。UV 由 uv_min/uv_max 指定——纯色块/散图
//! 全图 = (0,0)-(1,1)；atlas sprite = 子区。返回 SOA 四表，
//! 与 `NodePayload::Mesh` 同形。

use crate::scene::node::Rect;

/// 生成一个 quad 的 verts/uvs/colors/indices。
///
/// - 4 顶点（左上 → 右上 → 右下 → 左下，CCW）。
/// - UV 由 uv_min/uv_max 指定：TL→(umin,vmin) TR→(umax,vmin) BR→(umax,vmax) BL→(umin,vmax)。
///   纯色块/散图全图 = [0,0],[1,1]；atlas sprite = 子区。
/// - 4 顶点同色（quad 单色）。
/// - 两个三角形：`(0,1,2)` + `(0,2,3)`。
///
/// 用于 Container/Button 背景色块、Image 占位贴图 quad。
///
/// 返回 SOA 四表（verts/uvs/colors/indices），与 `NodePayload::Mesh` 字段同形，
/// 调用方直接解构装 payload。不为 clippy 折叠成 type 别名（单一调用点，别名徒增间接）。
#[allow(clippy::type_complexity)]
pub fn quad(
    rect: &Rect,
    color: [f32; 4],
    uv_min: [f32; 2],
    uv_max: [f32; 2],
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>) {
    let verts = vec![
        [rect.x, rect.y],
        [rect.x + rect.w, rect.y],
        [rect.x + rect.w, rect.y + rect.h],
        [rect.x, rect.y + rect.h],
    ];
    let (umin, vmin) = (uv_min[0], uv_min[1]);
    let (umax, vmax) = (uv_max[0], uv_max[1]);
    let uvs = vec![[umin, vmin], [umax, vmin], [umax, vmax], [umin, vmax]];
    let colors = vec![color; 4];
    let indices = vec![0, 1, 2, 0, 2, 3];
    (verts, uvs, colors, indices)
}

/// 生成圆角矩形的 verts/uvs/colors/indices（SOA 四表，与 quad 同形）。
///
/// - 三角扇：中心点 + 4 角圆弧顶点，三角形 (0, i, i+1) 连接，末尾回 1 闭合。
/// - radii = [TL, TR, BR, BL]，每角 (h, v) 像素半径（本函数内钳制）。
/// - UV 线性映射：顶点归一化位置 (pos-rect.xy)/rect.size × (uv_max-uv_min) + uv_min。
///   与 fit_uv 共存：uv_min/uv_max 即 fit_uv 算出的子区。
/// - 产 design y-down 坐标；v 翻转由调用点交换 uv v 处理（同 quad）。
/// - 退化（w/h≤0）走 quad fallback。
#[allow(clippy::type_complexity)]
pub fn rounded_rect(
    rect: &Rect,
    color: [f32; 4],
    radii: &[(f32, f32); 4],
    uv_min: [f32; 2],
    uv_max: [f32; 2],
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>) {
    let (w, h) = (rect.w, rect.h);
    if w <= 0.0 || h <= 0.0 {
        return quad(rect, color, uv_min, uv_max);
    }
    // 改进 1：CSS 按边缩放钳制（vs fgui per-corner min）。两邻角半径和不超过边长，等比缩放；
    // 只缩不放（min(1.0) 兜底）；防负 max(0.0)。
    let (tl, tr, br, bl) = (radii[0], radii[1], radii[2], radii[3]);
    let scale = 1.0_f32
        .min(w / (tl.0 + tr.0).max(1e-6))
        .min(w / (bl.0 + br.0).max(1e-6))
        .min(h / (tl.1 + bl.1).max(1e-6))
        .min(h / (tr.1 + br.1).max(1e-6))
        .min(1.0);
    let scale_r = |r: (f32, f32)| -> (f32, f32) {
        ((r.0 * scale).max(0.0), (r.1 * scale).max(0.0))
    };
    let tl = scale_r(tl);
    let tr = scale_r(tr);
    let br = scale_r(br);
    let bl = scale_r(bl);

    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let (umin, vmin) = (uv_min[0], uv_min[1]);
    let (umax, vmax) = (uv_max[0], uv_max[1]);

    let mut verts: Vec<[f32; 2]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    // 中心点（索引 0）
    let cx = rect.x + w / 2.0;
    let cy = rect.y + h / 2.0;
    verts.push([cx, cy]);
    uvs.push([lerp(umin, umax, 0.5), lerp(vmin, vmax, 0.5)]);
    colors.push(color);

    // 改进 2：角序 TL→TR→BR→BL（CSS 视觉序）。
    // 起始角 TL=π, TR=-π/2, BR=0, BL=π/2（逆时针 design y-down）。圆心 = 角顶点内缩 (rx,ry)。
    let corners: [(f32, f32, [f32; 2], f32); 4] = [
        (tl.0, tl.1, [rect.x + tl.0,         rect.y + tl.1],         std::f32::consts::PI),
        (tr.0, tr.1, [rect.x + w - tr.0,     rect.y + tr.1],         -std::f32::consts::FRAC_PI_2),
        (br.0, br.1, [rect.x + w - br.0,     rect.y + h - br.1],     0.0),
        (bl.0, bl.1, [rect.x + bl.0,         rect.y + h - bl.1],     std::f32::consts::FRAC_PI_2),
    ];
    for (rx, ry, center, start) in corners {
        if rx <= 0.0 || ry <= 0.0 {
            // 直角：单顶点 = 角顶点（照 fgui radius==0 分支）。
            let px = center[0] + start.cos() * rx;
            let py = center[1] + start.sin() * ry;
            verts.push([px, py]);
            uvs.push([lerp(umin, umax, (px - rect.x) / w), lerp(vmin, vmax, (py - rect.y) / h)]);
            colors.push(color);
            continue;
        }
        // 自适应分段（照搬 fgui：ceil(π·max(rx,ry)/8)+1，最小 2）
        let sides = ((std::f32::consts::PI * rx.max(ry) / 8.0).ceil() as i32 + 1).max(2);
        let delta = std::f32::consts::FRAC_PI_2 / sides as f32;
        for j in 0..=sides {
            let a = if j == sides {
                start + std::f32::consts::FRAC_PI_2  // 末段精度锁（照 fgui）
            } else {
                start + delta * j as f32
            };
            let px = center[0] + a.cos() * rx;
            let py = center[1] + a.sin() * ry;
            verts.push([px, py]);
            uvs.push([lerp(umin, umax, (px - rect.x) / w), lerp(vmin, vmax, (py - rect.y) / h)]);
            colors.push(color);
        }
    }
    // 三角扇：(0, i, i+1)，末尾回 1 闭合
    let n = verts.len() as u32;
    let mut indices: Vec<u32> = Vec::new();
    for i in 1..n {
        let next = if i + 1 < n { i + 1 } else { 1 };
        indices.extend_from_slice(&[0, i, next]);
    }
    (verts, uvs, colors, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_four_verts_two_tris() {
        let (v, _uvs, _col, i) = quad(&Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        }, [1.0; 4], [0.0, 0.0], [1.0, 1.0]);
        assert_eq!(v.len(), 4);
        assert_eq!(i.len(), 6);
    }

    #[test]
    fn quad_verts_at_corners() {
        // 左上 → 右上 → 右下 → 左下，CCW。
        let (v, _, _, _) = quad(
            &Rect {
                x: 1.0,
                y: 2.0,
                w: 3.0,
                h: 4.0,
            },
            [0.0; 4],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        assert_eq!(v[0], [1.0, 2.0]); // 左上 = (x, y)
        assert_eq!(v[1], [4.0, 2.0]); // 右上 = (x+w, y)
        assert_eq!(v[2], [4.0, 6.0]); // 右下 = (x+w, y+h)
        assert_eq!(v[3], [1.0, 6.0]); // 左下 = (x, y+h)
    }

    #[test]
    fn quad_uv_uses_min_max() {
        // 传 [0,0],[1,1] → 全图占位（TL=0,0 / BR=1,1）。
        let (_, uvs, _, _) = quad(&Rect::default(), [0.0; 4], [0.0, 0.0], [1.0, 1.0]);
        assert_eq!(uvs[0], [0.0, 0.0]);
        assert_eq!(uvs[2], [1.0, 1.0]);
    }

    #[test]
    fn quad_uv_subregion() {
        // atlas sprite 子区：TL→(umin,vmin) TR→(umax,vmin) BR→(umax,vmax) BL→(umin,vmax)。
        let (_, uvs, _, _) = quad(&Rect::default(), [0.0; 4], [0.25, 0.5], [0.75, 1.0]);
        assert_eq!(uvs[0], [0.25, 0.5]); // TL
        assert_eq!(uvs[1], [0.75, 0.5]); // TR
        assert_eq!(uvs[2], [0.75, 1.0]); // BR
        assert_eq!(uvs[3], [0.25, 1.0]); // BL
    }

    #[test]
    fn quad_colors_uniform() {
        let (_, _, colors, _) = quad(&Rect::default(), [0.5; 4], [0.0, 0.0], [1.0, 1.0]);
        for c in &colors {
            assert_eq!(*c, [0.5; 4]);
        }
    }

    #[test]
    fn rounded_rect_zero_radius_acts_as_rect() {
        // 全 0 直角：仍是三角扇，但所有弧顶点退化为角顶点 → 中心+4 角 = 5 顶点
        let (v, _uvs, _col, idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 },
            [1.0; 4],
            &[(0.0, 0.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        assert_eq!(v.len(), 5, "全 0：中心 + 4 角顶点");
        assert_eq!(idx.len(), 12, "4 三角形 × 3 索引");
    }

    #[test]
    fn rounded_rect_vertex_count_scales_with_radius() {
        // r=8, 80×80：sides = ceil(π·8/8)+1 = 5 → 每角 6 顶点(0..=5) × 4 角 + 1 中心 = 25
        let (v, _uvs, _col, _idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 },
            [1.0; 4],
            &[(8.0, 8.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        assert_eq!(v.len(), 25, "1 中心 + 4 角 × (5+1) 顶点");
    }

    #[test]
    fn rounded_rect_clamps_oversized_radius() {
        // 改进 1：r=40, 60×40 rect——四边约束取最紧：h/(tl.1+bl.1)=40/80=0.5 → scale=0.5 → r=20
        // TL 圆心 = (20, 20)，最左弧点 x = 20-20 = 0（贴 rect 左边）
        let (v, _uvs, _col, _idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 60.0, h: 40.0 },
            [1.0; 4],
            &[(40.0, 40.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        let xs: Vec<f32> = v.iter().map(|p| p[0]).collect();
        let min_x = xs.iter().cloned().fold(f32::INFINITY, f32::min);
        assert!((min_x - 0.0).abs() < 1e-3, "钳制后最左顶点贴 rect.x=0，得 {}", min_x);
        let max_x = xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!((max_x - 60.0).abs() < 1e-3, "最右顶点贴 rect.x+w=60，得 {}", max_x);
    }

    #[test]
    fn rounded_rect_ellipse_radii() {
        // 椭圆角 (20,10)：TL 圆心 (20,10)，弧顶点 = center + (cos·20, sin·10)
        // 起始角 TL=π：cos(π)=-1, sin(π)=0 → 首弧点 = (20-20, 10+0) = (0, 10)
        let (v, _uvs, _col, _idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 },
            [1.0; 4],
            &[(20.0, 10.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        // v[1] = 第一个弧顶点 = TL 角起始 (0, 10)
        assert!((v[1][0] - 0.0).abs() < 1e-3, "TL 首弧点 x=0，得 {}", v[1][0]);
        assert!((v[1][1] - 10.0).abs() < 1e-3, "TL 首弧点 y=10，得 {}", v[1][1]);
    }

    #[test]
    fn rounded_rect_uv_linear_mapping() {
        // rect=(0,0,80,80), uv=[0,0]-[1,1] → 中心顶点 UV=(0.5,0.5)
        let (_v, uvs, _col, _idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 80.0, h: 80.0 },
            [1.0; 4],
            &[(8.0, 8.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        assert!((uvs[0][0] - 0.5).abs() < 1e-3, "中心 UV.x=0.5");
        assert!((uvs[0][1] - 0.5).abs() < 1e-3, "中心 UV.y=0.5");
    }

    #[test]
    fn rounded_rect_degenerate_rect_falls_back_to_quad() {
        // w=0 退化 → 走 quad（4 顶点）
        let (v, _uvs, _col, idx) = rounded_rect(
            &Rect { x: 0.0, y: 0.0, w: 0.0, h: 80.0 },
            [1.0; 4],
            &[(8.0, 8.0); 4],
            [0.0, 0.0], [1.0, 1.0],
        );
        assert_eq!(v.len(), 4, "退化走 quad");
        assert_eq!(idx.len(), 6);
    }
}
