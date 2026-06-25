//! v1d.4 GTween-lite：tween 引擎（TweenManager + Ease/TweenProp）。
//! 动 opacity / transform(translate·scale·rotate) / 颜色(bg·text)。
//! replace-override：动画值覆盖 ResolvedStyle 读取点（None 退回 CSS）。

/// 可动属性。u8 值与 FFI / C# enum 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TweenProp {
    Opacity = 0,
    Translate = 1,
    Scale = 2,
    Rotation = 3,
    BgColor = 4,
    TextColor = 5,
}

/// 每个 prop 的 lerp 分量数（start/end [f32;4] 取前 N 个）。
pub fn prop_value_size(prop: TweenProp) -> u8 {
    match prop {
        TweenProp::Opacity | TweenProp::Rotation => 1,
        TweenProp::Translate | TweenProp::Scale => 2,
        TweenProp::BgColor | TweenProp::TextColor => 4,
    }
}

/// easing 子集（10 个）。u8 值与 FFI / C# enum 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Ease {
    Linear = 0,
    QuadIn = 1,
    QuadOut = 2,
    QuadInOut = 3,
    CubicIn = 4,
    CubicOut = 5,
    CubicInOut = 6,
    BackIn = 7,
    BackOut = 8,
    BackInOut = 9,
}

const OVERSHOOT: f32 = 1.70158;

impl Ease {
    /// t∈[0,dur] → [0,1]。dur<=0 直接返 1（调用方已钳 tt<=dur，这里防除零）。
    pub fn evaluate(self, t: f32, dur: f32) -> f32 {
        if dur <= 0.0 {
            return 1.0;
        }
        match self {
            Ease::Linear => t / dur,
            Ease::QuadIn => { let t = t / dur; t * t }
            Ease::QuadOut => { let t = t / dur; -t * (t - 2.0) }
            Ease::QuadInOut => {
                let t = t / (dur * 0.5);
                if t < 1.0 { 0.5 * t * t }
                else { let t = t - 1.0; -0.5 * (t * (t - 2.0) - 1.0) }
            }
            Ease::CubicIn => { let t = t / dur; t * t * t }
            Ease::CubicOut => { let t = t / dur - 1.0; t * t * t + 1.0 }
            Ease::CubicInOut => {
                let t = t / (dur * 0.5);
                if t < 1.0 { 0.5 * t * t * t }
                else { let t = t - 2.0; 0.5 * (t * t * t + 2.0) }
            }
            Ease::BackIn => { let t = t / dur; t * t * ((OVERSHOOT + 1.0) * t - OVERSHOOT) }
            Ease::BackOut => { let t = t / dur - 1.0; t * t * ((OVERSHOOT + 1.0) * t + OVERSHOOT) + 1.0 }
            Ease::BackInOut => {
                let s = OVERSHOOT * 1.525;
                let t = t / (dur * 0.5);
                if t < 1.0 { 0.5 * (t * t * ((s + 1.0) * t - s)) }
                else { let t = t - 2.0; 0.5 * (t * t * ((s + 1.0) * t + s) + 2.0) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prop_value_size_mapping() {
        assert_eq!(prop_value_size(TweenProp::Opacity), 1);
        assert_eq!(prop_value_size(TweenProp::Rotation), 1);
        assert_eq!(prop_value_size(TweenProp::Translate), 2);
        assert_eq!(prop_value_size(TweenProp::Scale), 2);
        assert_eq!(prop_value_size(TweenProp::BgColor), 4);
        assert_eq!(prop_value_size(TweenProp::TextColor), 4);
    }

    #[test]
    fn ease_endpoints_are_0_and_1() {
        let dur = 1.0;
        for ease in [
            Ease::Linear, Ease::QuadIn, Ease::QuadOut, Ease::QuadInOut,
            Ease::CubicIn, Ease::CubicOut, Ease::CubicInOut,
            Ease::BackIn, Ease::BackOut, Ease::BackInOut,
        ] {
            assert!((ease.evaluate(0.0, dur)).abs() < 1e-5, "{:?}@0 != 0", ease);
            assert!((ease.evaluate(dur, dur) - 1.0).abs() < 1e-5, "{:?}@dur != 1", ease);
        }
    }

    #[test]
    fn ease_dur_zero_returns_1() {
        assert_eq!(Ease::Linear.evaluate(0.5, 0.0), 1.0);
    }

    #[test]
    fn cubic_in_below_linear_below_cubic_out_at_mid() {
        // t=0.5,dur=1：CubicIn(0.125) < Linear(0.5) < CubicOut(0.875)
        let lin = Ease::Linear.evaluate(0.5, 1.0);
        let cin = Ease::CubicIn.evaluate(0.5, 1.0);
        let cout = Ease::CubicOut.evaluate(0.5, 1.0);
        assert!(cin < lin && lin < cout, "CubicIn({}) < Linear({}) < CubicOut({})", cin, lin, cout);
        assert!((cin - 0.125).abs() < 1e-5);
        assert!((cout - 0.875).abs() < 1e-5);
    }

    #[test]
    fn back_out_overshoots_above_1_mid() {
        // BackOut 中段 >1（overshoot）；约 t≈0.6 处达峰 ~1.1
        let mut max_v = 0.0f32;
        for i in 0..100 {
            let t = i as f32 / 100.0;
            let v = Ease::BackOut.evaluate(t, 1.0);
            if v > max_v { max_v = v; }
        }
        assert!(max_v > 1.0, "BackOut 中段须 >1（overshoot），实达 {}", max_v);
    }

    #[test]
    fn back_in_undershoots_below_0_early() {
        // BackIn 初段 <0（反向 overshoot）
        let v = Ease::BackIn.evaluate(0.1, 1.0);
        assert!(v < 0.0, "BackIn 初段须 <0，得 {}", v);
    }
}
