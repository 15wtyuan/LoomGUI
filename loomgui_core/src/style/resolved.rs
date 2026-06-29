use serde::{Deserialize, Serialize};
use taffy::style::LengthPercentage;
use taffy::style::Style as TaffyStyle;
use taffy::FlexDirection;

/// CSS overflow 轴模式（替旧 `overflow_hidden: bool`）。
/// `#[repr(u8)]` 保证 FFI/序列化稳定，`Default = Visible` 零回归旧 `overflow_hidden=false`。
/// Scroll/Auto 的物理/手势由 scroll 模块实现；本 enum 仅承载语义值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum OverflowMode {
    #[default]
    Visible = 0,
    Hidden = 1,
    Scroll = 2,
    Auto = 3,
}

/// CSS background-size 三档（v1 围栏子集）。
/// `#[repr(u8)]` 保证序列化稳定；`Default = Stretch`（100% 语义，未设时拉伸填满，非 CSS auto）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum BackgroundSize {
    #[default]
    Stretch = 0,  // 100% / 未设：UV 0..1 拉伸填满
    Cover = 1,    // 铺满裁剪（scale=max，UV 内收取子区中央）
    Contain = 2,  // 完整放入留白（scale=min，UV 外扩，子区外透明透出底色）
}

/// CSS border-radius 单角半径（v1.2）。
/// (h, v) = (水平, 垂直) 半径，存 CSS 原始值（px/%），渲染期 resolve 成像素。
/// `/` 省略时 v = h（正圆角）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CornerRadius {
    pub h: LengthPercentage,  // 水平半径
    pub v: LengthPercentage,  // 垂直半径
}
impl Default for CornerRadius {
    fn default() -> Self { Self { h: LengthPercentage::Length(0.0), v: LengthPercentage::Length(0.0) } }
}

/// CSS border-radius 四角半径（v1.2）。corners 序 [TL, TR, BR, BL]（CSS 1~4 值展开序）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct BorderRadius {
    pub corners: [CornerRadius; 4],
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
    /// CSS background-image url 路径（已去 url() 包裹 + 引号），None = 无背景图。
    pub background_image: Option<String>,
    /// CSS background-size 模式。默认 Stretch。
    pub background_size: BackgroundSize,
    /// CSS border-radius 四角半径（v1.2）。默认全 0（直角）。
    pub border_radius: BorderRadius,
    pub border_color: Option<[f32; 4]>,
    pub border_width: f32,
    pub opacity: f32,
    /// overflow 两轴模式（替 `overflow_hidden: bool`）。Default 双轴 Visible。
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
    /// layout 在 flex 排序前消费。默认 0 = DOM 顺序。
    pub order: i32,
    /// pointer-events:auto=true / none=false（命中门控）。默认 true。
    pub touchable: bool,
    /// CSS transform 解析产物（Affine2 矩阵，含多函数复合剪切）。默认 identity。
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
        // div 永远是 flex 容器，默认 flex-direction: column。
        // taffy Style::DEFAULT 是 Row，这里改默认为 Column。
        // CSS 显式声明 flex-direction 时，style::mapping::apply_decl 的对应分支
        // 无条件覆盖 ts.flex_direction——故显式声明永远胜出（写在 row 即 row）。
        let mut taffy_style = TaffyStyle::DEFAULT;
        taffy_style.flex_direction = FlexDirection::Column;
        Self {
            taffy_style,
            background_color: None,
            background_image: None,
            background_size: BackgroundSize::Stretch,
            border_radius: BorderRadius::default(),
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
        // div 默认 flex-direction: column（taffy DEFAULT 是 row，必须显式覆盖）
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
        s.background_image = Some("icons/home.png".to_string());
        s.background_size = BackgroundSize::Cover;
        s.border_radius = BorderRadius {
            corners: [
                CornerRadius { h: LengthPercentage::Length(12.0), v: LengthPercentage::Length(12.0) },
                CornerRadius { h: LengthPercentage::Length(0.0), v: LengthPercentage::Length(0.0) },
                CornerRadius { h: LengthPercentage::Percent(0.25), v: LengthPercentage::Percent(0.25) },
                CornerRadius { h: LengthPercentage::Length(4.0), v: LengthPercentage::Length(2.0) },
            ],
        };

        let bytes = bincode::serialize(&s).expect("serialize");
        let back: ResolvedStyle = bincode::deserialize(&bytes).expect("deserialize");

        assert_eq!(back, s, "全字段经 bincode round-trip 应相等");
    }

    #[test]
    fn background_image_size_default() {
        let s = ResolvedStyle::default();
        assert_eq!(s.background_image, None, "默认无背景图");
        assert_eq!(s.background_size, BackgroundSize::Stretch, "默认 Stretch（100% 语义）");
    }

    #[test]
    fn background_size_bincode_roundtrip() {
        let mut s = ResolvedStyle::default();
        s.background_size = BackgroundSize::Contain;
        s.background_image = Some("a.png".into());
        let bytes = bincode::serialize(&s).unwrap();
        let back: ResolvedStyle = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.background_size, BackgroundSize::Contain);
        assert_eq!(back.background_image.as_deref(), Some("a.png"));
        assert_eq!(back, s, "新字段 round-trip 全等");
    }

    #[test]
    fn border_radius_default_is_zero() {
        let s = ResolvedStyle::default();
        // 默认四角全 Length(0)（直角，零回归）
        for c in &s.border_radius.corners {
            assert_eq!(c.h, LengthPercentage::Length(0.0), "默认水平半径 0");
            assert_eq!(c.v, LengthPercentage::Length(0.0), "默认垂直半径 0");
        }
    }

    #[test]
    fn border_radius_bincode_roundtrip() {
        let mut s = ResolvedStyle::default();
        // 非默认：TL=(8px,8px) 正圆角，TR=(10px,5px) 椭圆角
        s.border_radius = BorderRadius {
            corners: [
                CornerRadius { h: LengthPercentage::Length(8.0), v: LengthPercentage::Length(8.0) },
                CornerRadius { h: LengthPercentage::Length(10.0), v: LengthPercentage::Length(5.0) },
                CornerRadius { h: LengthPercentage::Percent(0.5), v: LengthPercentage::Percent(0.5) },
                CornerRadius { h: LengthPercentage::Length(0.0), v: LengthPercentage::Length(0.0) },
            ],
        };
        let bytes = bincode::serialize(&s).unwrap();
        let back: ResolvedStyle = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.border_radius, s.border_radius, "border_radius 经 bincode round-trip 应相等");
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
