use taffy::style::Style as TaffyStyle;

#[derive(Debug, Clone)]
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
    /// img 的 src（v0 不加载，→ 占位 tex_id）
    pub img_src: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl Default for ResolvedStyle {
    fn default() -> Self {
        Self {
            taffy_style: TaffyStyle::DEFAULT,
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
            img_src: None,
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
    }
}
