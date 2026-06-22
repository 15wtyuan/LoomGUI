use serde::{Deserialize, Serialize};
use taffy::style::Style as TaffyStyle;
use taffy::FlexDirection;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedStyle {
    /// taffy 布局字段（flex/padding/margin/size/min/max/gap/position 等）
    pub taffy_style: TaffyStyle,
    /// 视觉字段（不进 taffy，渲染层消费）
    pub background_color: Option<[f32; 4]>, // rgba 0..1
    pub border_color: Option<[f32; 4]>,
    pub border_width: f32,
    pub opacity: f32,
    pub overflow_hidden: bool,
    pub color: [f32; 4],
    pub font_size: f32,
    pub font_family: Option<String>,
    pub font_weight: u16,
    pub text_align: TextAlign,
    pub line_height: f32, // 单位倍数（1.5 = 1.5x font-size），0 = normal
    pub letter_spacing: f32,
    pub white_space_nowrap: bool,
    /// flex 顺序（CSS `order`）。taffy 0.5 Style 无此字段，存在这里由
    /// Task 6 layout 在 flex 排序前消费。默认 0 = DOM 顺序。
    pub order: i32,
    /// pointer-events:auto=true / none=false（v1c.1 命中门控）。默认 true。
    pub touchable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl Default for ResolvedStyle {
    fn default() -> Self {
        // §4.1：div 永远是 flex 容器，默认 flex-direction: column。
        // taffy Style::DEFAULT 是 Row，这里改默认为 Column。
        // CSS 显式声明 flex-direction 时，style::mapping::apply_decl 的对应分支
        // 无条件覆盖 ts.flex_direction——故显式声明永远胜出（写在 row 即 row）。
        let mut taffy_style = TaffyStyle::DEFAULT;
        taffy_style.flex_direction = FlexDirection::Column;
        Self {
            taffy_style,
            background_color: None,
            border_color: None,
            border_width: 0.0,
            opacity: 1.0,
            overflow_hidden: false,
            color: [0.0, 0.0, 0.0, 1.0],
            font_size: 16.0,
            font_family: None,
            font_weight: 400,
            text_align: TextAlign::Left,
            line_height: 0.0,
            letter_spacing: 0.0,
            white_space_nowrap: false,
            order: 0,
            touchable: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_sane() {
        let s = ResolvedStyle::default();
        assert_eq!(s.opacity, 1.0);
        assert_eq!(s.font_size, 16.0);
        assert!(!s.overflow_hidden);
        // §4.1：div 默认 flex-direction: column（taffy DEFAULT 是 row，必须显式覆盖）
        assert_eq!(s.taffy_style.flex_direction, taffy::FlexDirection::Column);
    }

    #[test]
    fn resolved_style_bincode_roundtrip_preserves_all_fields() {
        // 构造一个各字段都非默认的 ResolvedStyle（覆盖 taffy 字段 + 视觉字段）。
        let mut s = ResolvedStyle::default();
        s.taffy_style.flex_direction = taffy::FlexDirection::Row;
        s.taffy_style.padding = taffy::geometry::Rect::length(7.0);
        s.background_color = Some([0.1, 0.2, 0.3, 0.4]);
        s.border_color = Some([0.5, 0.6, 0.7, 0.8]);
        s.border_width = 3.0;
        s.opacity = 0.5;
        s.overflow_hidden = true;
        s.color = [1.0, 0.0, 0.0, 1.0];
        s.font_size = 48.0;
        s.font_family = Some("DejaVuSans".to_string());
        s.font_weight = 700;
        s.text_align = TextAlign::Center;
        s.line_height = 1.5;
        s.letter_spacing = 2.0;
        s.white_space_nowrap = true;
        s.order = 5;

        let bytes = bincode::serialize(&s).expect("serialize");
        let back: ResolvedStyle = bincode::deserialize(&bytes).expect("deserialize");

        assert_eq!(back, s, "全字段经 bincode round-trip 应相等");
    }

    #[test]
    fn default_touchable_is_true() {
        assert!(ResolvedStyle::default().touchable, "touchable 默认 true（pointer-events:auto）");
    }

    #[test]
    fn touchable_bincode_roundtrip() {
        let mut s = ResolvedStyle::default();
        s.touchable = false;
        let bytes = bincode::serialize(&s).unwrap();
        let back: ResolvedStyle = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.touchable, false);
        assert_eq!(back, s, "加字段后全字段 round-trip 仍相等");
    }
}
