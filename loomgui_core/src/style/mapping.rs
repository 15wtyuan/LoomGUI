use crate::style::resolved::{BackgroundSize, BorderRadius, CornerRadius, OverflowMode, ResolvedStyle, TextAlign};
use taffy::geometry::{Rect, Size};
use taffy::style::{Dimension, LengthPercentage, LengthPercentageAuto};

/// px → Dimension::Length(f32)；% → LengthPercentage::Percent；auto → Auto
pub fn parse_length(s: &str) -> LengthPercentageAuto {
    let s = s.trim();
    if s == "auto" {
        return LengthPercentageAuto::Auto;
    }
    parse_lp(s).into()
}

pub fn parse_lp(s: &str) -> LengthPercentage {
    let s = s.trim();
    if let Some(pct) = s.strip_suffix('%') {
        if let Ok(v) = pct.trim().parse::<f32>() {
            return LengthPercentage::Percent(v / 100.0);
        }
    }
    if let Some(px) = s.strip_suffix("px") {
        if let Ok(v) = px.trim().parse::<f32>() {
            return LengthPercentage::Length(v);
        }
    }
    // 裸数字当 px
    if let Ok(v) = s.parse::<f32>() {
        return LengthPercentage::Length(v);
    }
    LengthPercentage::Length(0.0)
}

pub fn parse_dimension(s: &str) -> Dimension {
    let s = s.trim();
    if s == "auto" {
        return Dimension::Auto;
    }
    match parse_lp(s) {
        LengthPercentage::Length(v) => Dimension::Length(v),
        LengthPercentage::Percent(v) => Dimension::Percent(v),
    }
}

/// 1~4 值展开四向（top right bottom left）
pub fn parse_four(s: &str) -> [f32; 4] {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let p = |i: usize| -> f32 {
        parts
            .get(i)
            .and_then(|x| {
                x.strip_suffix("px")
                    .unwrap_or(x)
                    .trim()
                    .parse::<f32>()
                    .ok()
            })
            .unwrap_or(0.0)
    };
    match parts.len() {
        1 => {
            let v = p(0);
            [v, v, v, v]
        }
        2 => {
            let v = p(0);
            let h = p(1);
            [v, h, v, h]
        }
        3 => [p(0), p(1), p(2), p(1)],
        _ => [p(0), p(1), p(2), p(3)],
    }
}

/// 解析 border-radius 1~4 值（每值 px 或 %）→ [LengthPercentage;4]（TL,TR,BR,BL）。
/// 与 parse_four 同序，但保留 %。任一值非法（auto/inherit/initial/非 px-% 数字）→ None
/// （CSS：整条声明无效）。
fn parse_radius_group(s: &str) -> Option<[LengthPercentage; 4]> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let p = |i: usize| -> Option<LengthPercentage> {
        let tok = parts.get(i)?.trim();
        if tok == "auto" || tok == "inherit" || tok == "initial" {
            return None;
        }
        // parse_lp 对垃圾（如 "abc"）静默返回 Length(0)，需额外校验：
        // 合法 token = 裸数字 / 数字px / 数字%
        let num_part = tok.trim_end_matches("px").trim_end_matches('%');
        if num_part.trim().parse::<f32>().is_err() {
            return None;
        }
        Some(parse_lp(tok))
    };
    let v0 = p(0)?;
    Some(match parts.len() {
        1 => [v0, v0, v0, v0],
        2 => [v0, p(1)?, v0, p(1)?],
        3 => [v0, p(1)?, p(2)?, p(1)?],
        _ => [v0, p(1)?, p(2)?, p(3)?],
    })
}

pub fn parse_color(s: &str) -> Option<[f32; 4]> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
    } else {
        None
    }
}

/// 解析 CSS `background-image: url(...)` 值，提取括号内路径（去可选引号 + 首尾空格）。
/// 支持 `url(x)` / `url("x")` / `url('x')` / `url( x )`。非 url() 格式或空 → None。
pub fn parse_url(value: &str) -> Option<String> {
    let v = value.trim();
    let inner = v.strip_prefix("url(")?.strip_suffix(")")?;
    let inner = inner.trim();
    let len = inner.len();
    if len == 0 { return None; }
    // 去首尾配对引号
    let path = if len >= 2
        && ((inner.starts_with('"') && inner.ends_with('"'))
            || (inner.starts_with('\'') && inner.ends_with('\'')))
    {
        &inner[1..len - 1]
    } else {
        inner
    };
    let path = path.trim();
    if path.is_empty() { None } else { Some(path.to_string()) }
}

use crate::style::resolved::LocalTransform;
use crate::transform::{self, Affine2};

/// 解析 CSS `transform` 声明值为累积 Affine2 矩阵。
/// 支持 translate(px,px)/rotate(deg)/scale(num[,num])；skew/matrix()/%/3D 静默跳过。
/// 多函数从左到右 = 矩阵左乘累积（CSS 语义：最左函数最外层）。
pub fn parse_transform(value: &str) -> LocalTransform {
    let mut m = transform::IDENTITY;
    for (name, args) in iter_transform_funcs(value.trim()) {
        if let Some(fm) = func_to_matrix(name, args) {
            m = transform::mul(&m, &fm);
        }
    }
    LocalTransform { matrix: m }
}

/// 拆 "translate(10px,20px) rotate(45deg)" → [("translate","10px,20px"),("rotate","45deg")]。
fn iter_transform_funcs(s: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 跳空白
        while i < bytes.len() && bytes[i].is_ascii_whitespace() { i += 1; }
        if i >= bytes.len() { break; }
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'-') { i += 1; }
        let name = &s[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() { i += 1; }
        if i >= bytes.len() || bytes[i] != b'(' { break; }
        i += 1; // skip '('
        let args_start = i;
        while i < bytes.len() && bytes[i] != b')' { i += 1; }
        let args = &s[args_start..i];
        if i < bytes.len() { i += 1; } // skip ')'
        if !name.is_empty() { out.push((name, args)); }
    }
    out
}

/// 单函数 → Affine2。围栏外函数返 None（跳过）。
fn func_to_matrix(name: &str, args: &str) -> Option<Affine2> {
    let parts: Vec<&str> = args.split(',').map(|p| p.trim()).collect();
    match name {
        "translate" => {
            // translate 只支持 px，拒 %
            let x = parse_px(parts.first().copied().unwrap_or("0"))?;
            let y = parse_px(parts.get(1).copied().unwrap_or("0"))?;
            Some(transform::from_translate(x, y))
        }
        "rotate" => {
            let deg = parts.first().copied().unwrap_or("0");
            let deg = deg.trim_end_matches("deg").trim().parse::<f32>().ok()?;
            Some(transform::from_rotate(deg.to_radians()))
        }
        "scale" => {
            let sx = parts.first().copied().unwrap_or("1").parse::<f32>().ok()?;
            let sy = parts.get(1).copied().unwrap_or(&sx.to_string()).parse::<f32>().ok()?;
            Some(transform::from_scale(sx, sy))
        }
        _ => None, // skew/matrix3d/... 围栏外
    }
}

/// overflow 值 → OverflowMode。未知值返回 None（宽松忽略，不报错）。
fn parse_overflow(value: &str) -> Option<OverflowMode> {
    match value.trim() {
        "visible" => Some(OverflowMode::Visible),
        "hidden" => Some(OverflowMode::Hidden),
        "scroll" => Some(OverflowMode::Scroll),
        "auto" => Some(OverflowMode::Auto),
        _ => None,
    }
}

/// 把一条 declaration 应用到 style（覆盖对应字段）。返回是否被识别。
pub fn apply_decl(style: &mut ResolvedStyle, prop: &str, value: &str) -> bool {
    let ts = &mut style.taffy_style;
    match prop.trim() {
        "width" => {
            ts.size.width = parse_dimension(value);
            true
        }
        "height" => {
            ts.size.height = parse_dimension(value);
            true
        }
        "min-width" => {
            ts.min_size.width = parse_dimension(value);
            true
        }
        "min-height" => {
            ts.min_size.height = parse_dimension(value);
            true
        }
        "max-width" => {
            ts.max_size.width = parse_dimension(value);
            true
        }
        "max-height" => {
            ts.max_size.height = parse_dimension(value);
            true
        }
        "padding" => {
            let [t, r, b, l] = parse_four(value);
            ts.padding = Rect {
                left: LengthPercentage::Length(l),
                right: LengthPercentage::Length(r),
                top: LengthPercentage::Length(t),
                bottom: LengthPercentage::Length(b),
            };
            true
        }
        "margin" => {
            let [t, r, b, l] = parse_four(value);
            ts.margin = Rect {
                left: LengthPercentageAuto::Length(l),
                right: LengthPercentageAuto::Length(r),
                top: LengthPercentageAuto::Length(t),
                bottom: LengthPercentageAuto::Length(b),
            };
            true
        }
        "border" | "border-width" => {
            let [t, r, b, l] = parse_four(value);
            ts.border = Rect {
                left: LengthPercentage::Length(l),
                right: LengthPercentage::Length(r),
                top: LengthPercentage::Length(t),
                bottom: LengthPercentage::Length(b),
            };
            // 同时填视觉 border_width（取 top 作为单值，渲染描边用）
            style.border_width = t;
            true
        }
        "border-radius" => {
            // 语法：<len>{1,4} [ / <len>{1,4} ]?  —— / 前水平半径，/ 后垂直半径（省略=同水平）
            let (h_group, v_group) = match value.split_once('/') {
                Some((h, v)) => (h, v),
                None => (value, value),  // 无 / → 垂直 = 水平（正圆角）
            };
            let h = match parse_radius_group(h_group) {
                Some(g) => g,
                None => return false,
            };
            let v = match parse_radius_group(v_group) {
                Some(g) => g,
                None => return false,
            };
            style.border_radius = BorderRadius {
                corners: [
                    CornerRadius { h: h[0], v: v[0] },  // TL
                    CornerRadius { h: h[1], v: v[1] },  // TR
                    CornerRadius { h: h[2], v: v[2] },  // BR
                    CornerRadius { h: h[3], v: v[3] },  // BL
                ],
            };
            true
        }
        "gap" => {
            let f = parse_four(value);
            ts.gap = Size {
                width: LengthPercentage::Length(f[1]),
                height: LengthPercentage::Length(f[0]),
            };
            true
        }
        "flex-direction" => {
            ts.flex_direction = match value.trim() {
                "row" => taffy::FlexDirection::Row,
                "row-reverse" => taffy::FlexDirection::RowReverse,
                "column-reverse" => taffy::FlexDirection::ColumnReverse,
                _ => taffy::FlexDirection::Column,
            };
            true
        }
        "flex-wrap" => {
            ts.flex_wrap = match value.trim() {
                "wrap" => taffy::FlexWrap::Wrap,
                _ => taffy::FlexWrap::NoWrap,
            };
            true
        }
        "justify-content" => {
            ts.justify_content = Some(parse_justify(value));
            true
        }
        "align-items" => {
            ts.align_items = Some(parse_align(value));
            true
        }
        "align-self" => {
            ts.align_self = Some(parse_align(value));
            true
        }
        "flex-grow" => {
            ts.flex_grow = value.trim().parse::<f32>().unwrap_or(0.0);
            true
        }
        "flex-shrink" => {
            ts.flex_shrink = value.trim().parse::<f32>().unwrap_or(1.0);
            true
        }
        "flex-basis" => {
            ts.flex_basis = parse_dimension(value);
            true
        }
        "display" => {
            ts.display = match value.trim() {
                "none" => taffy::Display::None,
                _ => taffy::Display::Flex,
            };
            true
        }
        "background-color" => {
            style.background_color = parse_color(value);
            true
        }
        "background-image" => {
            style.background_image = parse_url(value);
            style.background_image.is_some()
        }
        "background-size" => {
            style.background_size = match value.trim() {
                "cover" => BackgroundSize::Cover,
                "contain" => BackgroundSize::Contain,
                "100%" => BackgroundSize::Stretch,
                _ => return false,  // 围栏外值（auto/px/两值）静默忽略
            };
            true
        }
        "border-color" => {
            style.border_color = parse_color(value);
            true
        }
        "opacity" => {
            style.opacity = value
                .trim()
                .trim_end_matches('%')
                .parse::<f32>()
                .unwrap_or(1.0)
                .min(1.0);
            true
        }
        "overflow" => {
            // shorthand：双轴同值。未知值宽松忽略（不动既有字段，仍返回 true）。
            if let Some(m) = parse_overflow(value) {
                style.overflow_x = m;
                style.overflow_y = m;
            }
            true
        }
        "overflow-x" => {
            // longhand：单轴 x。后于 shorthand apply 即覆盖（CSS 同 specificity 源序后写者胜）。
            if let Some(m) = parse_overflow(value) {
                style.overflow_x = m;
            }
            true
        }
        "overflow-y" => {
            if let Some(m) = parse_overflow(value) {
                style.overflow_y = m;
            }
            true
        }
        "color" => {
            if let Some(c) = parse_color(value) {
                style.color = c;
            }
            true
        }
        "font-size" => {
            style.font_size = parse_px(value).unwrap_or(style.font_size);
            true
        }
        "font-family" => {
            style.font_family = Some(value.trim().to_string());
            true
        }
        "font-weight" => {
            style.font_weight = value.trim().parse::<u16>().unwrap_or(400);
            true
        }
        "text-align" => {
            style.text_align = match value.trim() {
                "center" => TextAlign::Center,
                "right" => TextAlign::Right,
                _ => TextAlign::Left,
            };
            true
        }
        "line-height" => {
            style.line_height = value
                .trim()
                .trim_end_matches("px")
                .parse::<f32>()
                .unwrap_or(0.0);
            true
        }
        "letter-spacing" => {
            style.letter_spacing = parse_px(value).unwrap_or(0.0);
            true
        }
        "white-space" => {
            style.white_space_nowrap = value.trim() == "nowrap";
            true
        }
        "aspect-ratio" => {
            if let Ok(v) = value.trim().parse::<f32>() {
                ts.aspect_ratio = Some(v);
            }
            true
        }
        "order" => {
            // taffy 0.5 Style 无 order 字段；存进 ResolvedStyle.order，
            // 由 layout 在 flex 排序前消费。非法值降级为 0。
            style.order = value.trim().parse::<i32>().unwrap_or(0);
            true
        }
        "pointer-events" => {
            // auto/默认=true（可命中），none=false（跳过自身，继续测子——CSS 语义）
            style.touchable = value.trim() != "none";
            true
        }
        "transform" => {
            style.transform = parse_transform(value);
            true
        }
        _ => false, // 装饰属性静默忽略
    }
}

/// "10px" → 10.0；"10%" → None（拒 %）；"10" → 10.0（容错无单位）。
fn parse_px(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.ends_with('%') { return None; }
    s.trim_end_matches("px").trim().parse::<f32>().ok()
}

fn parse_justify(v: &str) -> taffy::JustifyContent {
    // JustifyContent 是 AlignContent 的类型别名（taffy 0.5），用全路径构造
    match v.trim() {
        "center" => taffy::AlignContent::Center,
        "flex-end" => taffy::AlignContent::FlexEnd,
        "space-between" => taffy::AlignContent::SpaceBetween,
        "space-around" => taffy::AlignContent::SpaceAround,
        "space-evenly" => taffy::AlignContent::SpaceEvenly,
        _ => taffy::AlignContent::FlexStart,
    }
}
fn parse_align(v: &str) -> taffy::AlignItems {
    match v.trim() {
        "center" => taffy::AlignItems::Center,
        "flex-end" => taffy::AlignItems::FlexEnd,
        "stretch" => taffy::AlignItems::Stretch,
        "baseline" => taffy::AlignItems::Baseline,
        _ => taffy::AlignItems::FlexStart,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taffy::style::LengthPercentage;
    #[test]
    fn parse_length_px_pct_auto() {
        assert!(matches!(parse_lp("100px"), LengthPercentage::Length(100.0)));
        assert!(matches!(parse_lp("50%"), LengthPercentage::Percent(0.5)));
    }
    /// `width:auto` 必须解析成 `Dimension::Auto`（fit-content），
    /// 不能 fallback 到 `Length(0.0)`（→ img rect=(0,0) 不渲染）。
    #[test]
    fn parse_dimension_auto_is_auto_not_zero() {
        use taffy::style::Dimension;
        assert!(matches!(parse_dimension("auto"), Dimension::Auto), "auto → Auto");
        assert!(matches!(parse_dimension("80px"), Dimension::Length(80.0)));
        assert!(matches!(parse_dimension("50%"), Dimension::Percent(0.5)));
    }
    #[test]
    fn four_value_expand() {
        assert_eq!(parse_four("4px"), [4.0, 4.0, 4.0, 4.0]);
        assert_eq!(parse_four("4px 8px"), [4.0, 8.0, 4.0, 8.0]);
    }
    #[test]
    fn color_hex() {
        let c = parse_color("#ff0000").unwrap();
        assert_eq!(c, [1.0, 0.0, 0.0, 1.0]);
    }
    #[test]
    fn apply_width_and_bg() {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "width", "100px"));
        assert!(apply_decl(&mut s, "background-color", "#00ff00"));
        assert!(s.background_color == Some([0.0, 1.0, 0.0, 1.0]));
        assert!(apply_decl(&mut s, "border-radius", "4px")); // v1.2 起解析（非装饰忽略）
    }
    #[test]
    fn order_is_stored() {
        // 合法值：存进 ResolvedStyle.order，不再静默丢弃
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "order", "2"));
        assert_eq!(s.order, 2);
        // 非法值：降级为 0（不 panic、不污染）
        let mut s2 = ResolvedStyle::default();
        assert!(apply_decl(&mut s2, "order", "abc"));
        assert_eq!(s2.order, 0);
        // 负值也接受（CSS order 允许负）
        let mut s3 = ResolvedStyle::default();
        assert!(apply_decl(&mut s3, "order", "-1"));
        assert_eq!(s3.order, -1);
    }

    #[test]
    fn pointer_events_none_sets_touchable_false() {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "pointer-events", "none"));
        assert!(!s.touchable, "pointer-events:none → touchable=false");
    }

    #[test]
    fn pointer_events_auto_keeps_touchable_true() {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "pointer-events", "auto"));
        assert!(s.touchable, "pointer-events:auto → touchable=true");
    }

    #[test]
    fn overflow_shorthand_sets_both_axes() {
        // overflow:scroll → overflow_x=overflow_y=Scroll
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "overflow", "scroll"));
        assert_eq!(s.overflow_x, OverflowMode::Scroll);
        assert_eq!(s.overflow_y, OverflowMode::Scroll);
    }

    #[test]
    fn overflow_shorthand_all_values() {
        for (val, mode) in [
            ("visible", OverflowMode::Visible),
            ("hidden", OverflowMode::Hidden),
            ("scroll", OverflowMode::Scroll),
            ("auto", OverflowMode::Auto),
        ] {
            let mut s = ResolvedStyle::default();
            assert!(apply_decl(&mut s, "overflow", val), "overflow:{} 被识别", val);
            assert_eq!(s.overflow_x, mode, "overflow:{} → x", val);
            assert_eq!(s.overflow_y, mode, "overflow:{} → y", val);
        }
    }

    #[test]
    fn overflow_xy_longhand_overrides_shorthand() {
        // shorthand 先设双轴 hidden；longhand 后写 override 单轴
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "overflow", "hidden"));
        assert!(apply_decl(&mut s, "overflow-x", "auto"));
        assert_eq!(s.overflow_x, OverflowMode::Auto, "overflow-x longhand 覆盖");
        assert_eq!(s.overflow_y, OverflowMode::Hidden, "overflow-y 保持 shorthand");
    }

    #[test]
    fn overflow_xy_longhand_y_axis() {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "overflow", "visible"));
        assert!(apply_decl(&mut s, "overflow-y", "scroll"));
        assert_eq!(s.overflow_x, OverflowMode::Visible, "overflow-x 保持");
        assert_eq!(s.overflow_y, OverflowMode::Scroll, "overflow-y longhand");
    }

    #[test]
    fn overflow_unknown_value_silently_ignored() {
        // 未知值宽松忽略：既存字段不变（与现 overflow 解析风格一致）
        let mut s = ResolvedStyle::default();
        s.overflow_x = OverflowMode::Scroll;
        s.overflow_y = OverflowMode::Auto;
        assert!(apply_decl(&mut s, "overflow", "bogus"));
        assert_eq!(s.overflow_x, OverflowMode::Scroll, "未知 overflow 不动 x");
        assert_eq!(s.overflow_y, OverflowMode::Auto, "未知 overflow 不动 y");
        assert!(apply_decl(&mut s, "overflow-x", "nonsense"));
        assert_eq!(s.overflow_x, OverflowMode::Scroll, "未知 overflow-x 不动 x");
    }

    use super::parse_transform;
    use crate::transform::Affine2Ext;

    #[test]
    fn parse_single_translate() {
        let t = parse_transform("translate(10px, 20px)");
        let (x, y) = t.matrix.apply_point(0.0, 0.0);
        assert_eq!((x, y), (10.0, 20.0));
        assert!(t.matrix.is_pure_translation(), "纯 translate 是纯平移");
    }

    #[test]
    fn parse_single_rotate_radians() {
        let t = parse_transform("rotate(90deg)");
        // 90° 旋转：(1,0) → (0,1)
        let (x, y) = t.matrix.apply_point(1.0, 0.0);
        assert!(x.abs() < 1e-5 && (y - 1.0).abs() < 1e-5, "90deg rotate (1,0)→(0,1)");
    }

    #[test]
    fn parse_single_scale_uniform() {
        let t = parse_transform("scale(2)");
        let (x, y) = t.matrix.apply_point(1.0, 1.0);
        assert_eq!((x, y), (2.0, 2.0), "scale(2) 双轴");
    }

    #[test]
    fn parse_scale_non_uniform_compose_with_rotate_is_skew() {
        // scale(2,1) rotate(45deg)：复合矩阵非纯平移（剪切）
        let t = parse_transform("scale(2, 1) rotate(45deg)");
        assert!(!t.matrix.is_pure_translation(), "非均匀缩放∘旋转 = 剪切，非纯平移");
    }

    #[test]
    fn parse_unknown_functions_silently_skipped() {
        // skew/matrix() 围栏外 → 静默跳过；translate 仍生效
        let t = parse_transform("translate(5px, 0px) skew(10deg)");
        let (x, y) = t.matrix.apply_point(0.0, 0.0);
        assert_eq!((x, y), (5.0, 0.0), "skew 被跳过，translate 生效");
    }

    #[test]
    fn apply_decl_transform_sets_style() {
        use crate::style::resolved::ResolvedStyle;
        use crate::transform::Affine2Ext;
        let mut s = ResolvedStyle::default();
        let applied = super::apply_decl(&mut s, "transform", "rotate(45deg)");
        assert!(applied, "transform 被识别");
        assert!(!s.transform.matrix.is_pure_translation(), "rotate 写进 style.transform");
    }

    #[test]
    fn parse_url_extracts_path() {
        use super::parse_url;
        assert_eq!(parse_url("url(icons/home.png)"), Some("icons/home.png".into()));
        assert_eq!(parse_url("url(\"icons/home.png\")"), Some("icons/home.png".into()));
        assert_eq!(parse_url("url('icons/home.png')"), Some("icons/home.png".into()));
        assert_eq!(parse_url("url( icons/home.png )"), Some("icons/home.png".into()), "容忍空格");
        assert_eq!(parse_url("icons/home.png"), None, "非 url() 格式 → None");
        assert_eq!(parse_url("url()"), None, "空 url → None");
        assert_eq!(parse_url(""), None);
        // 自闭合引号回归测试：len < 2 被 len >= 2 guard 拦住，不应 panic
        assert_eq!(parse_url("url(')"), Some("'".to_string()), "自闭合单引号不 panic");
        assert_eq!(parse_url("url(\")"), Some("\"".to_string()), "自闭合双引号不 panic");
    }

    #[test]
    fn apply_background_image_sets_field() {
        use crate::style::resolved::BackgroundSize;
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "background-image", "url(icons/home.png)"));
        assert_eq!(s.background_image.as_deref(), Some("icons/home.png"));
        // 无图时默认 Stretch 不变
        assert_eq!(s.background_size, BackgroundSize::Stretch);
    }

    #[test]
    fn apply_background_size_three_modes() {
        use crate::style::resolved::BackgroundSize;
        for (val, mode) in [
            ("cover", BackgroundSize::Cover),
            ("contain", BackgroundSize::Contain),
            ("100%", BackgroundSize::Stretch),
        ] {
            let mut s = ResolvedStyle::default();
            assert!(apply_decl(&mut s, "background-size", val), "background-size:{} 被识别", val);
            assert_eq!(s.background_size, mode, "background-size:{} → {:?}", val, mode);
        }
    }

    #[test]
    fn apply_background_size_invalid_ignored() {
        // 围栏外值（auto/px/两值）→ 返回 false，不改默认 Stretch
        use crate::style::resolved::BackgroundSize;
        for val in ["auto", "50px", "100% 50%", "cover contain"] {
            let mut s = ResolvedStyle::default();
            assert!(!apply_decl(&mut s, "background-size", val), "{} 围栏外 → false", val);
            assert_eq!(s.background_size, BackgroundSize::Stretch, "{} 不改默认", val);
        }
    }

    #[test]
    fn parse_radius_group_one_value() {
        let g = parse_radius_group("8px").unwrap();
        assert_eq!(g, [parse_lp("8px"); 4]);
    }

    #[test]
    fn parse_radius_group_two_values() {
        let g = parse_radius_group("4px 12px").unwrap();
        // [v0, v1, v0, v1]（TL/BR=v0, TR/BL=v1）
        assert_eq!(g, [parse_lp("4px"), parse_lp("12px"), parse_lp("4px"), parse_lp("12px")]);
    }

    #[test]
    fn parse_radius_group_percent() {
        let g = parse_radius_group("50%").unwrap();
        assert_eq!(g, [parse_lp("50%"); 4]);
    }

    #[test]
    fn parse_radius_group_auto_rejected() {
        // auto/inherit/initial → None（CSS 无效，不落 Length(0)）
        assert!(parse_radius_group("auto").is_none());
        assert!(parse_radius_group("inherit").is_none());
        assert!(parse_radius_group("initial").is_none());
        assert!(parse_radius_group("8px auto").is_none());  // 混入 auto → 整组 None
    }

    #[test]
    fn parse_radius_group_garbage_rejected() {
        assert!(parse_radius_group("4px abc").is_none());
        assert!(parse_radius_group("").is_none());
    }

    #[test]
    fn apply_border_radius_single() {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "border-radius", "8px"));
        for c in &s.border_radius.corners {
            assert_eq!(c.h, parse_lp("8px"));
            assert_eq!(c.v, parse_lp("8px"));  // 无 / → v = h
        }
    }

    #[test]
    fn apply_border_radius_ellipse_syntax() {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "border-radius", "10px / 5px"));
        for c in &s.border_radius.corners {
            assert_eq!(c.h, parse_lp("10px"), "水平半径 10");
            assert_eq!(c.v, parse_lp("5px"), "垂直半径 5");
        }
    }

    #[test]
    fn apply_border_radius_percent() {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, "border-radius", "50%"));
        for c in &s.border_radius.corners {
            assert_eq!(c.h, LengthPercentage::Percent(0.5));
            assert_eq!(c.v, LengthPercentage::Percent(0.5));
        }
    }

    #[test]
    fn apply_border_radius_invalid_returns_false() {
        let mut s = ResolvedStyle::default();
        assert!(!apply_decl(&mut s, "border-radius", "auto"));
        assert!(!apply_decl(&mut s, "border-radius", "8px / abc"));
        assert!(!apply_decl(&mut s, "border-radius", "8px /"));
        // 失败时不应改默认
        assert_eq!(s.border_radius, BorderRadius::default());
    }
}
