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
}
