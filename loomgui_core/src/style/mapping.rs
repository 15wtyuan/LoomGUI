use crate::style::resolved::{ResolvedStyle, TextAlign};
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
            style.overflow_hidden = value.trim() == "hidden";
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
            // taffy 0.5 Style 无 order 字段；视觉/布局顺序由 DOM 顺序决定。
            // 接受声明但不存储（v0 装饰性忽略）。
            true
        }
        _ => false, // 装饰属性静默忽略（§4.1）
    }
}

fn parse_px(s: &str) -> Option<f32> {
    let s = s.trim().trim_end_matches("px").trim();
    s.parse::<f32>().ok()
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
    #[test]
    fn parse_length_px_pct_auto() {
        assert!(matches!(parse_lp("100px"), LengthPercentage::Length(100.0)));
        assert!(matches!(parse_lp("50%"), LengthPercentage::Percent(0.5)));
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
        assert!(!apply_decl(&mut s, "border-radius", "4px")); // 装饰属性忽略
    }
}
