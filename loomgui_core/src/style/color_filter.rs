//! CSS filter → 4×5 颜色矩阵（照搬 fgui ColorFilter.cs）。
//! 矩阵行主序 20 float：[r0c0..r0c3, off0, r1c0..r1c3, off1, r2c0..r2c3, off2, r3c0..r3c3, off3]。
//! alpha 行恒 (0,0,0,1,0) → alpha 不变。

const LUMA_R: f32 = 0.299;
const LUMA_G: f32 = 0.587;
const LUMA_B: f32 = 0.114;

/// 单位矩阵（无 filter）。
pub const IDENTITY: [f32; 20] = [
    1.0, 0.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.0, 1.0, 0.0,
];

/// grayscale(1) = AdjustSaturation(-1)。
pub fn grayscale() -> [f32; 20] {
    // sat=0, invSat=1 → 行 r = (LUMA_r, LUMA_g, LUMA_b, 0, 0)
    let mut m = IDENTITY;
    m[0] = LUMA_R; m[1] = LUMA_G; m[2] = LUMA_B;
    m[5] = LUMA_R; m[6] = LUMA_G; m[7] = LUMA_B;
    m[10] = LUMA_R; m[11] = LUMA_G; m[12] = LUMA_B;
    m
}

/// brightness(n) = CSS 乘法 rgb×n（n=1 不变）。
/// 旧实现照搬 fgui AdjustBrightness(n-1)（加法 offset），与 CSS filter 乘法语义不符 → 改乘法。
pub fn brightness(n: f32) -> [f32; 20] {
    let mut m = IDENTITY;
    m[0] = n; m[6] = n; m[12] = n;
    m
}

/// contrast(n) = AdjustContrast(n-1)。s=n, o=128/255*(1-s)。
pub fn contrast(n: f32) -> [f32; 20] {
    let s = n;
    let o = 128.0 / 255.0 * (1.0 - s);
    let mut m = IDENTITY;
    m[0] = s; m[6] = s; m[12] = s;
    m[4] = o; m[9] = o; m[14] = o;
    m
}

/// saturate(n) = AdjustSaturation(n-1)。
pub fn saturate(n: f32) -> [f32; 20] {
    let sat = n;
    let inv = 1.0 - sat;
    let mut m = IDENTITY;
    m[0] = inv * LUMA_R + sat; m[1] = inv * LUMA_G;       m[2] = inv * LUMA_B;
    m[5] = inv * LUMA_R;       m[6] = inv * LUMA_G + sat; m[7] = inv * LUMA_B;
    m[10] = inv * LUMA_R;      m[11] = inv * LUMA_G;      m[12] = inv * LUMA_B + sat;
    m
}

/// hue-rotate(deg) = AdjustHue(deg/180)。
pub fn hue_rotate(deg: f32) -> [f32; 20] {
    let v = deg.to_radians();
    let (cos, sin) = (v.cos(), v.sin());
    let mut m = IDENTITY;
    // 照搬 fgui AdjustHue 行 0/1/2（行 3 alpha 不变）
    m[0] = LUMA_R + cos * (1.0 - LUMA_R) + sin * (-LUMA_R);
    m[1] = LUMA_G + cos * (-LUMA_G) + sin * (-LUMA_G);
    m[2] = LUMA_B + cos * (-LUMA_B) + sin * (1.0 - LUMA_B);
    m[5] = LUMA_R + cos * (-LUMA_R) + sin * 0.143;
    m[6] = LUMA_G + cos * (1.0 - LUMA_G) + sin * 0.14;
    m[7] = LUMA_B + cos * (-LUMA_B) + sin * (-0.283);
    m[10] = LUMA_R + cos * (-LUMA_R) + sin * (-(1.0 - LUMA_R));
    m[11] = LUMA_G + cos * (-LUMA_G) + sin * LUMA_G;
    m[12] = LUMA_B + cos * (1.0 - LUMA_B) + sin * LUMA_B;
    m
}

/// invert(1)。
pub fn invert() -> [f32; 20] {
    let mut m = IDENTITY;
    m[0] = -1.0; m[6] = -1.0; m[12] = -1.0;
    m[4] = 1.0; m[9] = 1.0; m[14] = 1.0;
    m
}

/// 矩阵相乘 a ∘ b（fgui ConcatValues）：result = a × b。
/// 4 行 × 5 列，行主序。result[r][c] = sum_k a[r][k] * b[k][c]（c<4），
/// c=4（offset 列）= sum_k a[r][k]*b[k][4] + a[r][4]。
pub fn concat(a: &[f32; 20], b: &[f32; 20]) -> [f32; 20] {
    let mut out = [0.0; 20];
    for r in 0..4 {
        for c in 0..5 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += a[r * 5 + k] * b[k * 5 + c];
            }
            if c == 4 {
                sum += a[r * 5 + 4];
            }
            out[r * 5 + c] = sum;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_unit() {
        let m = IDENTITY;
        // 行主序：对角线 1，其余 0
        assert_eq!(m[0], 1.0);  assert_eq!(m[6], 1.0);  assert_eq!(m[12], 1.0);  assert_eq!(m[18], 1.0);
        assert_eq!(m[1], 0.0);  assert_eq!(m[4], 0.0);  // off-diag + offset 列
    }

    #[test]
    fn grayscale_matches_fgui_saturation_neg1() {
        // fgui AdjustSaturation(-1)：sat=0, invSat=1 → 行 0 = (LUMA_R, LUMA_G, LUMA_B, 0, 0)
        let m = grayscale();
        assert!((m[0] - 0.299).abs() < 1e-4, "grayscale[0]=LUMA_R");
        assert!((m[1] - 0.587).abs() < 1e-4, "grayscale[1]=LUMA_G");
        assert!((m[2] - 0.114).abs() < 1e-4, "grayscale[2]=LUMA_B");
        // 行 1/2 同（灰度 = 三通道按 luma 混合）
        assert!((m[5] - 0.299).abs() < 1e-4);
        assert!((m[10] - 0.299).abs() < 1e-4);
        // alpha 行 (0,0,0,1,0)
        assert_eq!(m[18], 1.0);
    }

    #[test]
    fn invert_negates_rgb() {
        let m = invert();
        assert_eq!(m[0], -1.0);  assert_eq!(m[6], -1.0);  assert_eq!(m[12], -1.0);
        assert_eq!(m[4], 1.0);   assert_eq!(m[9], 1.0);   assert_eq!(m[14], 1.0);  // offset +1
        assert_eq!(m[18], 1.0);  // alpha 不变
    }

    #[test]
    fn concat_identity_is_noop() {
        let m = grayscale();
        let id = concat(&m, &IDENTITY);
        assert_eq!(id, m, "concat 任意矩阵 × IDENTITY = 原矩阵");
    }

    #[test]
    fn concat_grayscale_twice_equals_once() {
        // 灰化幂等：grayscale ∘ grayscale = grayscale
        let once = grayscale();
        let twice = concat(&once, &once);
        for i in 0..20 {
            assert!((twice[i] - once[i]).abs() < 1e-4, "grayscale 幂等 [{}]", i);
        }
    }

    #[test]
    fn brightness_multiplies() {
        // CSS brightness(1.2) = rgb×1.2（乘法）：对角 1.2，offset 0。
        // fgui AdjustBrightness 是加法 (n-1) → 与 CSS 不符，改乘法。
        let m = brightness(1.2);
        assert!((m[0] - 1.2).abs() < 1e-4, "brightness 对角=1.2");
        assert!((m[6] - 1.2).abs() < 1e-4);
        assert!((m[12] - 1.2).abs() < 1e-4);
        assert!(m[4].abs() < 1e-4, "brightness offset=0");
    }
}
