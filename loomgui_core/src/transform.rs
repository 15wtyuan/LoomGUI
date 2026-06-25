//! 2D 仿射矩阵（v1d.3 transform 核心）。被 LocalTransform / world_transforms / RenderNode / hit 共用。
//!
//! Affine2 = [a, b, c, d, tx, ty]，列主序含义：
//!   x' = a·x + c·y + tx
//!   y' = b·x + d·y + ty
//! 矩阵形如 [[a, c, tx], [b, d, ty], [0, 0, 1]]。

/// 2D 仿射 [a,b,c,d,tx,ty]。
pub type Affine2 = [f32; 6];

pub const IDENTITY: Affine2 = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

pub fn from_translate(tx: f32, ty: f32) -> Affine2 {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

pub fn from_scale(sx: f32, sy: f32) -> Affine2 {
    [sx, 0.0, 0.0, sy, 0.0, 0.0]
}

/// 绕原点逆时针旋转 rad 弧度的仿射。
/// x' = cos·x - sin·y, y' = sin·x + cos·y → a=cos,b=sin,c=-sin,d=cos。
pub fn from_rotate(rad: f32) -> Affine2 {
    let (s, c) = rad.sin_cos();
    [c, s, -s, c, 0.0, 0.0]
}

/// 矩阵乘 self ∘ other（先应用 other，再应用 self）。
/// M = self, N = other → M·N（点先经 N 再经 M）。
pub fn mul(self_m: &Affine2, other: &Affine2) -> Affine2 {
    let (a1, b1, c1, d1, tx1, ty1) = (self_m[0], self_m[1], self_m[2], self_m[3], self_m[4], self_m[5]);
    let (a2, b2, c2, d2, tx2, ty2) = (other[0], other[1], other[2], other[3], other[4], other[5]);
    [
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * tx2 + c1 * ty2 + tx1,
        b1 * tx2 + d1 * ty2 + ty1,
    ]
}

/// 矩阵 × 点。
pub fn apply_point(m: &Affine2, x: f32, y: f32) -> (f32, f32) {
    (
        m[0] * x + m[2] * y + m[4],
        m[1] * x + m[3] * y + m[5],
    )
}

/// 仿射逆（det≠0）。用伴随矩阵法。
pub fn inverse(m: &Affine2) -> Affine2 {
    let (a, b, c, d, tx, ty) = (m[0], m[1], m[2], m[3], m[4], m[5]);
    let det = a * d - b * c;
    let inv_det = 1.0 / det;
    // 线性部分逆 = 1/det · [[d,-c],[-b,a]]；平移部分 = -inv_lin · (tx,ty)
    let inv_a = d * inv_det;
    let inv_b = -b * inv_det;
    let inv_c = -c * inv_det;
    let inv_d = a * inv_det;
    let inv_tx = -(inv_a * tx + inv_c * ty);
    let inv_ty = -(inv_b * tx + inv_d * ty);
    [inv_a, inv_b, inv_c, inv_d, inv_tx, inv_ty]
}

/// 是否单位矩阵。
pub fn is_identity(m: &Affine2) -> bool {
    m[0] == 1.0 && m[1] == 0.0 && m[2] == 0.0 && m[3] == 1.0 && m[4] == 0.0 && m[5] == 0.0
}

/// 是否纯平移（a=1,b=0,c=0,d=1）→ 后端走 TRS 路径、参与 merge。
/// 用 epsilon 容错（解析/累计浮点误差）。
pub fn is_pure_translation(m: &Affine2) -> bool {
    const EPS: f32 = 1e-6;
    (m[0] - 1.0).abs() < EPS && m[1].abs() < EPS && m[2].abs() < EPS && (m[3] - 1.0).abs() < EPS
}

/// 给 Affine2 配套的扩展 trait，让调用处写 `m.mul(...)` / `m.apply_point(...)` 更顺。
pub trait Affine2Ext {
    fn mul(self, other: Affine2) -> Affine2;
    fn apply_point(self, x: f32, y: f32) -> (f32, f32);
    fn inverse(self) -> Affine2;
    fn is_identity(self) -> bool;
    fn is_pure_translation(self) -> bool;
}
impl Affine2Ext for Affine2 {
    fn mul(self, other: Affine2) -> Affine2 { mul(&self, &other) }
    fn apply_point(self, x: f32, y: f32) -> (f32, f32) { apply_point(&self, x, y) }
    fn inverse(self) -> Affine2 { inverse(&self) }
    fn is_identity(self) -> bool { is_identity(&self) }
    fn is_pure_translation(self) -> bool { is_pure_translation(&self) }
}

// 测试用：`use super::*` 导入 IDENTITY const + from_* fn + Affine2Ext trait；
// 测试里直接 `IDENTITY` / `from_translate(...)` / `.mul()` 即可（无 `` 前缀——
// `Affine2` 是 type alias 不能同名建 mod，故用 free fn + trait）。

#[cfg(test)]
mod tests {
    use super::*;
    const FRAC_45: f32 = std::f32::consts::FRAC_PI_4;

    #[test]
    fn identity_is_pure_translation_and_identity() {
        assert!(IDENTITY.is_identity());
        assert!(IDENTITY.is_pure_translation());
    }

    #[test]
    fn translate_apply_point() {
        let t = from_translate(10.0, 20.0);
        let (x, y) = t.apply_point(1.0, 2.0);
        assert_eq!((x, y), (11.0, 22.0));
        assert!(t.is_pure_translation());
    }

    #[test]
    fn rotate_45_applies_correctly() {
        let r = from_rotate(FRAC_45);
        let (x, y) = r.apply_point(1.0, 0.0);
        assert!((x - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        assert!((y - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        assert!(!r.is_pure_translation(), "rotate 非纯平移");
    }

    #[test]
    fn scale_non_uniform_is_not_pure_translation() {
        let s = from_scale(2.0, 1.0);
        assert!(!s.is_pure_translation());
    }

    #[test]
    fn mul_compose_translate_then_rotate() {
        // T(10,0) ∘ R(45°)：先 rotate 再 translate（CSS 右到左：rotate 在右）
        let m = from_translate(10.0, 0.0).mul(from_rotate(FRAC_45));
        // (1,0) → R(45) → (√2/2,√2/2) → T(10,0) → (10+√2/2, √2/2)
        let (x, y) = m.apply_point(1.0, 0.0);
        assert!((x - (10.0 + std::f32::consts::FRAC_1_SQRT_2)).abs() < 1e-5);
        assert!((y - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    }

    #[test]
    fn inverse_roundtrip() {
        let m = from_translate(10.0, 20.0)
            .mul(from_rotate(0.3))
            .mul(from_scale(2.0, 0.5));
        let inv = m.inverse();
        let (x, y) = m.apply_point(3.0, 4.0);
        let (bx, by) = inv.apply_point(x, y);
        assert!((bx - 3.0).abs() < 1e-4);
        assert!((by - 4.0).abs() < 1e-4);
    }

    #[test]
    fn inverse_of_skew_composite_exists() {
        // 非均匀缩放 ∘ 旋转 = 剪切矩阵，det≠0 仍可逆
        let skew = from_scale(2.0, 1.0).mul(from_rotate(FRAC_45));
        let (x, y) = skew.apply_point(1.0, 1.0);
        let inv = skew.inverse();
        let (bx, by) = inv.apply_point(x, y);
        assert!((bx - 1.0).abs() < 1e-4 && (by - 1.0).abs() < 1e-4, "剪切矩阵可逆");
    }
}
