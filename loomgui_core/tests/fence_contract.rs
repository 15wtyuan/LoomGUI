//! 围栏契约测试 = LoomGUI 围栏权威真相源（docs/design/fence.md 是人类副本）。
//! 三类断言：
//!   A. 元素围栏：围栏外标签报错（parse_html），白名单接受。
//!   B. 支持属性：apply_decl 返回 true + ResolvedStyle 字段变化。
//!   C. 围栏外属性：apply_decl 返回 false + 布局字段不变（静默忽略）。
//! 改 apply_decl / FENCE_TAGS / selector 必须同步本测试 + fence.md。

use loomgui_core::parse::css::parse_css;
use loomgui_core::parse::dom::parse_html;
use loomgui_core::style::mapping::apply_decl;
use loomgui_core::style::resolved::ResolvedStyle;
use taffy::Display;

// ── A. 元素围栏 ──────────────────────────────────────────────────

#[test]
fn fence_tags_whitelist_accepted() {
    // FENCE_TAGS = div/span/img/button（砍 l-container，与 div 同映射冗余）。
    for tag in ["div", "span", "img", "button"] {
        let html = format!("<{tag}></{tag}>");
        assert!(parse_html(&html).is_ok(), "<{tag}> 应被围栏接受");
    }
}

#[test]
fn fence_out_tags_rejected() {
    // 围栏外标签一律报错，不降级。l-container 砍后是围栏外（用 div）。
    for tag in ["video", "input", "b", "section", "p", "ul", "l-container"] {
        let html = format!("<{tag}></{tag}>");
        assert!(parse_html(&html).is_err(), "<{tag}> 应被围栏拒绝");
    }
}

// ── B. 支持属性生效（apply_decl 返回 true）──────────────────────

#[test]
fn supported_layout_props_return_true() {
    let cases = [
        ("display", "flex"),
        ("flex-direction", "row"),
        ("flex-wrap", "wrap"),
        ("gap", "10px"),
        ("justify-content", "center"),
        ("align-items", "center"),
        ("width", "100px"),
        ("padding", "8px"),
        ("margin", "4px"),
        ("aspect-ratio", "1.5"),
        ("order", "2"),
    ];
    for (prop, val) in cases {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, prop, val), "支持属性 {prop}:{val} 应返回 true");
    }
}

#[test]
fn supported_visual_props_return_true() {
    let cases = [
        ("background-color", "#5fb2c4"),
        ("background-image", "url(\"a.png\")"),
        ("background-size", "cover"),
        ("border-radius", "4px"),
        ("opacity", "0.5"),
        ("overflow", "hidden"),
        ("color", "#e0e0e0"),
        ("font-size", "16px"),
        ("font-weight", "700"),
        ("text-align", "center"),
        ("white-space", "nowrap"),
        ("transform", "rotate(45deg)"),
        ("pointer-events", "none"),
        ("filter", "grayscale(1)"),
        ("border-image-slice", "10"),
    ];
    for (prop, val) in cases {
        let mut s = ResolvedStyle::default();
        assert!(apply_decl(&mut s, prop, val), "支持属性 {prop}:{val} 应返回 true");
    }
}

#[test]
fn background_size_rejects_two_values() {
    // background-size 只认 cover/contain/100%，拒两值如 "100% 50%"。
    let mut s = ResolvedStyle::default();
    assert!(!apply_decl(&mut s, "background-size", "100% 50%"),
        "background-size 两值应被拒（返回 false）");
}

#[test]
fn display_grid_falls_to_flex() {
    // display:grid 走 mapping.rs 非 none 分支 → Flex，返回 true。
    // taffy 无 grid，grid 写了等于 flex，AI 不可预测 → fence.md 标"禁写 grid"。
    let mut s = ResolvedStyle::default();
    let ok = apply_decl(&mut s, "display", "grid");
    assert!(ok, "display:grid 走非 none 分支返回 true（落 Flex）");
    assert_eq!(s.taffy_style.display, Display::Flex,
        "display:grid 应落到 Flex（taffy 无 grid）");
}

// ── C. 围栏外属性静默忽略（apply_decl 返回 false，布局字段不变）─────
// fence.md §2.4 / §3.3 标【推断·待测】转【实证】的关键项。
// AI 写了以为生效、实际无效 = 不可预测，围栏禁写，测试锁定"无效"行为。

#[test]
fn fence_out_props_return_false() {
    let cases: [(&str, &str); 10] = [
        ("position", "absolute"),
        ("float", "left"),
        ("align-content", "center"),
        ("cursor", "pointer"),
        ("clip-path", "circle(50%)"),
        ("background-position", "center"),
        ("background-repeat", "no-repeat"),
        ("transform-origin", "top left"),
        ("font-style", "italic"),
        ("border-style", "dashed"),
    ];
    for (prop, val) in cases {
        let mut s = ResolvedStyle::default();
        assert!(!apply_decl(&mut s, prop, val),
            "围栏外属性 {prop}:{val} 应返回 false（静默忽略）");
    }
}

#[test]
fn position_absolute_does_not_break_flow() {
    // position:absolute 写了不生效：apply_decl 返回 false，
    // taffy_style.position 保持默认 Relative（不脱离流）。
    // fence.md §0 纠正的"无 match ≠ 不支持"反例的核心锁定。
    let mut s = ResolvedStyle::default();
    let before = s.taffy_style.position;
    let applied = apply_decl(&mut s, "position", "absolute");
    assert!(!applied, "position:absolute 应返回 false（围栏外）");
    assert_eq!(s.taffy_style.position, before,
        "position 字段不变（保持默认 Relative，不脱离流）");
}

#[test]
fn transform_skew_does_not_apply() {
    // transform 只认 translate/rotate/scale，skew 显式跳过（mapping.rs:278）。
    // apply_decl("transform",...) 返回 true（进 match arm），但 transform 字段无变化。
    let mut s1 = ResolvedStyle::default();
    let applied = apply_decl(&mut s1, "transform", "skew(10deg,5deg)");
    assert!(applied, "skew 应进 transform arm 返回 true（no-op 但进 arm）");
    let s2 = ResolvedStyle::default();
    assert_eq!(s1.transform, s2.transform, "skew 不应改变 transform 字段");
}

#[test]
fn at_rule_media_skipped_by_parser() {
    // @media 被 AtRuleParser 默认拒（parse/css.rs:58-63），整块跳过不报错。
    let css = "@media (min-width: 600px) { .a { width: 100px; } }";
    let sheet = parse_css(css).expect("parse_css 不应 panic");
    // @media 块被跳过，sheet 里无 .a 规则。
    assert!(sheet.rules.is_empty(), "@media 块应被跳过，规则不进 StyleSheet");
}
