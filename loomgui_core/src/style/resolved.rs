use serde::{Deserialize, Serialize};
use taffy::style::Style as TaffyStyle;
use taffy::FlexDirection;

/// v1d.5：CSS overflow 轴模式（替旧 `overflow_hidden: bool`）。
/// `#[repr(u8)]` 保证 FFI/序列化稳定（坑 34），`Default = Visible` 零回归旧 `overflow_hidden=false`。
/// Scroll/Auto 的物理/手势由 v1d.5 T6/T7 实现；本 enum 仅承载语义值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum OverflowMode {
    #[default]
    Visible = 0,
    Hidden = 1,
    Scroll = 2,
    Auto = 3,
}

/// CSS transform 解析产物。内部存 Affine2 矩阵（非分解字段）——这样单节点
/// `scale(2,1) rotate(45deg)` 的复合剪切在解析期就保留，不因提取字段丢失。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LocalTransform {
    pub matrix: crate::transform::Affine2,
}
impl Default for LocalTransform {
    fn default() -> Self { Self { matrix: crate::transform::IDENTITY } }
}
impl LocalTransform {
    pub fn is_identity(&self) -> bool { crate::transform::is_identity(&self.matrix) }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedStyle {
    /// taffy 布局字段（flex/padding/margin/size/min/max/gap/position 等）
    pub taffy_style: TaffyStyle,
    /// 视觉字段（不进 taffy，渲染层消费）
    pub background_color: Option<[f32; 4]>, // rgba 0..1
    pub border_color: Option<[f32; 4]>,
    pub border_width: f32,
    pub opacity: f32,
    /// v1d.5：overflow 两轴模式（替 `overflow_hidden: bool`）。Default 双轴 Visible。
    pub overflow_x: OverflowMode,
    pub overflow_y: OverflowMode,
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
    /// v1d.3：CSS transform 解析产物（Affine2 矩阵，含多函数复合剪切）。默认 identity。
    pub transform: crate::style::LocalTransform,
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
            overflow_x: OverflowMode::Visible,
            overflow_y: OverflowMode::Visible,
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
            transform: LocalTransform::default(),
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
        assert_eq!(s.overflow_x, OverflowMode::Visible, "overflow_x 默认 Visible");
        assert_eq!(s.overflow_y, OverflowMode::Visible, "overflow_y 默认 Visible");
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
        s.overflow_x = OverflowMode::Hidden;
        s.overflow_y = OverflowMode::Hidden;
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

    #[test]
    fn local_transform_default_is_identity_matrix() {
        let t = LocalTransform::default();
        assert!(t.is_identity(), "默认 transform = identity 矩阵");
    }

    #[test]
    fn resolved_style_transform_bincode_roundtrip() {
        let mut s = ResolvedStyle::default();
        s.transform = LocalTransform { matrix: crate::transform::from_rotate(0.5) };
        let bytes = bincode::serialize(&s).expect("serialize");
        let back: ResolvedStyle = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back.transform.matrix, s.transform.matrix, "transform 经 bincode round-trip");
    }

    #[test]
    fn overflow_default_is_visible_both_axes() {
        let s = ResolvedStyle::default();
        assert_eq!(s.overflow_x, OverflowMode::Visible);
        assert_eq!(s.overflow_y, OverflowMode::Visible);
    }

    #[test]
    fn overflow_mode_is_one_byte() {
        assert_eq!(std::mem::size_of::<OverflowMode>(), 1);
    }

    #[test]
    fn overflow_hidden_bincode_roundtrip() {
        // 零回归：Hidden 经 bincode round-trip 不变（pkg 字段）
        let mut s = ResolvedStyle::default();
        s.overflow_x = OverflowMode::Hidden;
        s.overflow_y = OverflowMode::Scroll;
        let bytes = bincode::serialize(&s).expect("serialize");
        let back: ResolvedStyle = bincode::deserialize(&bytes).expect("deserialize");
        assert_eq!(back.overflow_x, OverflowMode::Hidden);
        assert_eq!(back.overflow_y, OverflowMode::Scroll);
        assert_eq!(back, s, "overflow 字段 round-trip 全等");
    }
}
