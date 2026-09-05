use crate::diagnostic::{Diagnostic, DiagnosticCode, LineMap};
use crate::ir::{IrNodeKind, IrTree};
use crate::schema::css::{
    find_css_prop, find_shorthand, parse_animation_value, parse_transition_value,
    validate_animation_value, CssValueParser,
};
use crate::schema::tag::{find_tag, DisplayDefault, SemanticKind};
use yio_core::style::mapping::apply_decl;
use yio_core::style::resolved::{DisplayMode, ResolvedStyle, TextAlign, TextDecoration};

/// 围栏外但常见的 CSS 属性 → 引导文案（说明 Yio 行为 + 建议怎么改）。
///
/// 这些属性围栏不支持，写了一律 error（`FenceUnknownCssProp`）阻断打包——
/// 但 error message 帮作者改到 Yio 等价写法，而非只说「不在围栏」。
/// 返回 `None` 的属性走通用文案。
///
/// 共享给 inline style（`css_resolve`）、fence gate（`fence_gate`）、外部
/// `<style>` 块（`css_rules`）三处 `FenceUnknownCssProp` 构造点，保证引导文案一致。
pub(crate) fn unsupported_hint(prop: &str) -> Option<&'static str> {
    Some(match prop {
        // 契约 = CSS 初始值 content-box（css-reference「padding adds to the set width/height」）。
        // #116：本文案曾写反（border-box 措辞）——与 core pin（c30b9945）矛盾八个月，误导消费侧 AI。
        "box-sizing" => "Yio uses the CSS default content-box model: padding adds to the set width/height (width:420px with padding:22px renders 464px wide). There is no border-box switch — remove this declaration and subtract padding from width/height yourself.",
        "visibility" => "Yio has no visibility:hidden. To hide an element use `display:none` (removes layout space) or `opacity:0` (keeps space).",
        "cursor" | "outline" | "user-select" | "object-fit" => {
            "not supported by fence — remove this declaration."
        }
        _ => return None,
    })
}

/// Resolve inline styles for all nodes in the tree.
///
/// Returns one `ResolvedStyle` per node, in node-index order.
/// Uses the existing `apply_decl` for value application, but validates
/// property names and keyword values against the CSS schema first.
pub fn resolve_inline_styles(tree: &IrTree) -> Vec<ResolvedStyle> {
    resolve_inline_styles_with_diags(tree, "<inline>", &LineMap::new("")).0
}

/// Resolve inline styles, also returning diagnostics for invalid CSS.
pub fn resolve_inline_styles_with_diags(
    tree: &IrTree,
    file: &str,
    line_map: &LineMap,
) -> (Vec<ResolvedStyle>, Vec<Diagnostic>) {
    let mut styles: Vec<ResolvedStyle> = (0..tree.nodes.len())
        .map(|_| ResolvedStyle::default())
        .collect();
    let mut diagnostics = Vec::new();

    for (idx, node) in tree.nodes.iter().enumerate() {
        let IrNodeKind::Element(el) = &node.kind else {
            continue;
        };

        // Apply DisplayDefault from schema (overrides ResolvedStyle::default,
        // whose display fields stay Flex — taffy's own DEFAULT is Flex).
        // Custom elements (hyphenated tags, not in TAGS) default to Block like
        // div: falling through to the Flex default makes the host a flex-row
        // container, so template roots without explicit width shrink to content
        // (browser block-level template roots fill the page instead).
        let spec = find_tag(&el.tag);
        let display_default = match (spec, el.tag.contains('-')) {
            (Some(spec), _) => Some(spec.display),
            (None, true) => Some(DisplayDefault::Block),
            (None, false) => None,
        };
        if let Some(display_default) = display_default {
            match display_default {
                DisplayDefault::Block => {
                    styles[idx].display_mode = DisplayMode::Block;
                    // Schema-level default for block tags (div/header/nav/p/...).
                    // Must set taffy Display::Block here too — otherwise the
                    // taffy_style.display field keeps its Flex default from
                    // ResolvedStyle::default() and explicit display:block in
                    // mapping.rs can't rescue plain <div> without inline style.
                    // Explicit display:flex/none in inline style still wins: this
                    // runs first, apply_decl overwrites later.
                    styles[idx].taffy_style.display = taffy::Display::Block;
                }
                DisplayDefault::Inline => {
                    styles[idx].display_mode = DisplayMode::Flex;
                    // inline -> flex for taffy compatibility; flex-direction
                    // stays Row (taffy default) per CSS standard.
                }
                DisplayDefault::None => {
                    styles[idx].display_mode = DisplayMode::None;
                    // Must set taffy Display::None here too — every core pruner
                    // (collect_display_none_subtree, taffy layout cut, hit-test
                    // via zero layout_rect) keys off taffy_style.display, not
                    // display_mode. Leaving it at taffy's Flex default (from
                    // ResolvedStyle::default) lets <template> subtrees get real
                    // layout and render/hit-test. Mirrors the Block arm above.
                    styles[idx].taffy_style.display = taffy::Display::None;
                }
            }
            // UA 样式表等价：button 默认 text-align: center（浏览器 UA 行为）。
            // Yio 无 UA 样式表概念——直接在 tag default 处硬编码。运行时
            // propagate_inherited 会把此值继承给 text 子节点（"Buy" 等居中）。
            // 同时 set INH_TEXT_ALIGN bit，把 UA 默认视为"显式声明"——防
            // propagate_inherited 用父（卡片/列表项）的 text-align 覆盖 button。
            // 用户显式 text-align 声明仍走 inline apply_decl 分支覆盖（CSS 级联）。
            if spec.is_some_and(|s| s.semantic == SemanticKind::Button) {
                styles[idx].text_align = TextAlign::Center;
                if let Some(bit) = yio_core::style::dynamic::inherited_bit("text-align") {
                    styles[idx].inherited_set.0 |= bit;
                }
                // UA 容器居中：button 默认 justify-content + align-items = center（CSS 浏览器 UA
                // 行为：button content 居中）。Bug B 只修 text-align center（
                // text *内部* 居中），但 button 作为 flex 容器在缺省 justify/align=flex-start/stretch
                // 时，text 子作为 flex item 仍从 padding-left 起——core dump 实证 text.x=266 而非
                // 居中 268.5。justify-content/align-items 非继承属性 → 无 INH bit，仅本节点生效，
                // 运行时 rematch 从 base_style 重起，UA 默认每帧稳定。
                styles[idx].taffy_style.justify_content = Some(taffy::JustifyContent::CENTER);
                styles[idx].taffy_style.align_items = Some(taffy::AlignItems::CENTER);
            }
            // UA 样式表等价（#74）：`<a>` 默认链接色 #0000EE + text-decoration:underline
            //（浏览器 UA 行为）。顺序即级联：tag 默认先烙，作者 inline style 声明在下方
            // style_attr 循环后应用即赢（CSS 作者 > UA）；class 规则走运行时 rematch 从
            // base_style 起应用，同样赢。color 是继承属性——同 button text-align 先例烙
            // INH_COLOR bit，防运行时 propagate_inherited 拿父值覆盖链接色；
            // text-decoration 不继承，无需 bit。
            if spec.is_some_and(|s| s.semantic == SemanticKind::Link) {
                styles[idx].color = [0.0, 0.0, 238.0 / 255.0, 1.0];
                if let Some(bit) = yio_core::style::dynamic::inherited_bit("color") {
                    styles[idx].inherited_set.0 |= bit;
                }
                styles[idx].text_decoration = TextDecoration::Underline;
            }
        }

        if let Some(style_attr) = el.attributes.iter().find(|a| a.name == "style") {
            // 本元素的 custom prop 声明收集（元素级环 warning 用）。
            let mut el_customs: Vec<(String, String)> = Vec::new();
            for decl in style_attr.value.split(';') {
                let decl = decl.trim();
                if decl.is_empty() {
                    continue;
                }
                let (prop, value) = match decl.split_once(':') {
                    Some((p, v)) => (p.trim(), v.trim()),
                    None => continue,
                };

                // `--*` 自定义属性（#11 三源之「行内 style」）：值近乎自由，不烘焙进
                // typed 字段——存 deferred_inline，运行时在 var 环境参与解析。
                if crate::var_check::is_custom_prop(prop) {
                    if let Some(msg) = crate::var_check::var_shape_error(value) {
                        diagnostics.push(Diagnostic::error(
                            DiagnosticCode::FenceBadCssValue,
                            msg,
                            line_map.source_location(node.span.start, file.to_string()),
                        ));
                    } else {
                        styles[idx]
                            .deferred_inline
                            .push(yio_core::style::dynamic::Declaration {
                                prop: prop.to_string(),
                                value: value.to_string(),
                            });
                        el_customs.push((prop.to_string(), value.to_string()));
                    }
                    continue;
                }

                let is_known = find_css_prop(prop).is_some() || find_shorthand(prop).is_some();
                if !is_known {
                    let hint = unsupported_hint(prop).unwrap_or(
                        "not supported by fence — remove or replace with a supported property.",
                    );
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::FenceUnknownCssProp,
                        format!("CSS property \"{}\": {}", prop, hint),
                        line_map.source_location(node.span.start, file.to_string()),
                    ));
                    continue;
                }

                // 含 var() 的行内值（#11）：终值运行时在 var 环境解析（prop 名仍须合法，
                // 值字面校验跳过、只做形状校验）。同存 deferred_inline——行内优先级
                // 运行时重放；inline_declared 位在此标记，class 规则运行时不覆盖它。
                if crate::var_check::value_has_var(value) {
                    if let Some(msg) = crate::var_check::var_shape_error(value) {
                        diagnostics.push(Diagnostic::error(
                            DiagnosticCode::FenceBadCssValue,
                            msg,
                            line_map.source_location(node.span.start, file.to_string()),
                        ));
                        continue;
                    }
                    styles[idx]
                        .deferred_inline
                        .push(yio_core::style::dynamic::Declaration {
                            prop: prop.to_string(),
                            value: value.to_string(),
                        });
                    if let Some(bit) = yio_core::style::dynamic::inline_bit(prop) {
                        styles[idx].inline_declared |= bit;
                    }
                    continue;
                }

                // 共享值域门（宽松吞值通道：颜色/overflow 简写与 longhand/filter/transform；
                // value_check 自带 shorthand 域映射）+ display:inline 语义警告。
                if let Some(msg) = crate::value_check::value_error(prop, value) {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::FenceBadCssValue,
                        msg,
                        line_map.source_location(node.span.start, file.to_string()),
                    ));
                    continue;
                }
                if let Some(note) = crate::value_check::display_inline_warning(value) {
                    diagnostics.push(Diagnostic::warning(
                        DiagnosticCode::FenceDisplayInline,
                        format!("CSS property \"display\": {note}"),
                        line_map.source_location(node.span.start, file.to_string()),
                    ));
                }

                if let Some(spec) = find_css_prop(prop) {
                    match &spec.parser {
                        CssValueParser::Keyword(allowed) => {
                            if !allowed.contains(&value) {
                                diagnostics.push(Diagnostic::error(
                                    DiagnosticCode::FenceBadCssValue,
                                    format!(
                                        "value \"{}\" is not valid for CSS property \"{}\" (allowed: {})",
                                        value,
                                        prop,
                                        allowed.join(" | ")
                                    ),
                                    line_map.source_location(node.span.start, file.to_string()),
                                ));
                                continue;
                            }
                        }
                        CssValueParser::Animation => {
                            // animation 简写：先校验（捕捉拼写错误），合法则解析存值
                            // （runtime KeyframePlayer 消费 base_style.animation）。
                            // 不调 apply_decl：fence 要先跑 validate 门（apply_decl 宽松解析无诊断），
                            // 解析本身委托 core `parse_animation`（与运行时 rematch 的 apply_decl
                            // "animation" arm 同一真相源）。
                            if !validate_animation_value(value) {
                                diagnostics.push(Diagnostic::error(
                                    DiagnosticCode::FenceBadCssValue,
                                    format!(
                                        "value \"{}\" is not valid for CSS property \"{}\"",
                                        value, prop
                                    ),
                                    line_map.source_location(node.span.start, file.to_string()),
                                ));
                            } else {
                                styles[idx].animation = parse_animation_value(value);
                            }
                            continue;
                        }
                        CssValueParser::Transition => {
                            // transition 简写解析存值（core transition 引擎读 base_style.transition）。
                            // 值结构宽松（parse 忽略未知 token），但属性域外声明要警告——
                            // 引擎驱动的通道全集见 value_check::TRANSITION_PROPS（与 core
                            // TweenProp 一一对应），域外属性浏览器会过渡、Yio 静默 snap
                            // （预览≠运行时）。
                            for msg in crate::value_check::transition_warnings(value) {
                                diagnostics.push(Diagnostic::warning(
                                    DiagnosticCode::FenceTransitionUnsupportedProp,
                                    msg,
                                    line_map.source_location(node.span.start, file.to_string()),
                                ));
                            }
                            styles[idx].transition = parse_transition_value(value);
                            continue;
                        }
                        _ => {}
                    }
                }

                // 纯整数域属性（z-index/order）严格校验：apply_decl 对它们宽松降级 0，
                // 围栏不静默降级——坏值在打包期报清（font-weight 等 Integer parser 属性
                // 接受关键字，不在此列）。
                if matches!(prop, "z-index" | "order") && value.parse::<i32>().is_err() {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::FenceBadCssValue,
                        format!(
                            "value \"{}\" is not valid for CSS property \"{}\" (integer required)",
                            value, prop
                        ),
                        line_map.source_location(node.span.start, file.to_string()),
                    ));
                    continue;
                }

                // If it returns false, the value failed to parse -- report it.
                if !apply_decl(&mut styles[idx], prop, value) {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::FenceBadCssValue,
                        format!(
                            "value \"{}\" is not valid for CSS property \"{}\"",
                            value, prop
                        ),
                        line_map.source_location(node.span.start, file.to_string()),
                    ));
                } else {
                    // inline 声明标记：记该属性由 inline style 声明，rematch 时 class 规则
                    // 不覆盖它（CSS inline > class）。INLINE_* 位覆盖继承与非继承（如 display）。
                    if let Some(bit) = yio_core::style::dynamic::inline_bit(prop) {
                        styles[idx].inline_declared |= bit;
                    }
                    // inline 可继承声明另 bake 进 inherited_set，避免运行时
                    // propagate_inherited 用父值覆盖子的 inline 声明。
                    if let Some(bit) = yio_core::style::dynamic::inherited_bit(prop) {
                        styles[idx].inherited_set.0 |= bit;
                    }
                }
            }
            // 元素级 custom prop 引用环 warning（#11 分层 fail-loud：同元素 style attr 内
            // 静态可见的环）。与 <style> 块级检查（css_rules）同一真相源 var_check。
            for msg in crate::var_check::custom_prop_cycle_warnings(
                el_customs.iter().map(|(p, v)| (p.as_str(), v.as_str())),
            ) {
                diagnostics.push(Diagnostic::warning(
                    DiagnosticCode::FenceCustomPropCycle,
                    msg,
                    line_map.source_location(node.span.start, file.to_string()),
                ));
            }
        }

        // flex-direction 默认 = CSS 初始值 row（ResolvedStyle::default() 已是 Row，
        // 同 taffy DEFAULT）。显式声明走 apply_decl 无条件覆盖。无需补偿。
    }

    (styles, diagnostics)
}

/// Private helper for tests: resolve without file/line_map (uses empty).
#[cfg(test)]
fn resolve_for_test(tree: &IrTree) -> Vec<ResolvedStyle> {
    resolve_inline_styles_with_diags(tree, "<inline>", &LineMap::new("")).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_builder::parse_html_to_ir;

    #[test]
    fn inline_style_applies_color() {
        let (tree, _) = parse_html_to_ir(r#"<div style="color:#ff0000"></div>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(styles[id.0].color, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn display_block_overrides_default() {
        let (tree, _) = parse_html_to_ir(r#"<div style="display:block"></div>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(styles[id.0].display_mode, DisplayMode::Block);
    }

    #[test]
    fn display_grid_reports_error() {
        let (tree, _) = parse_html_to_ir(r#"<div style="display:grid"></div>"#);
        let (_, diags) = resolve_inline_styles_with_diags(&tree, "test.html", &LineMap::new(""));
        assert!(diags
            .iter()
            .any(|d| d.code == DiagnosticCode::FenceBadCssValue));
    }

    #[test]
    fn flex_defaults_to_row_direction() {
        let (tree, _) = parse_html_to_ir(r#"<div style="display:flex"></div>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(
            styles[id.0].taffy_style.flex_direction,
            taffy::FlexDirection::Row
        );
    }

    #[test]
    fn explicit_flex_direction_preserved() {
        let (tree, _) =
            parse_html_to_ir(r#"<div style="display:flex; flex-direction:column"></div>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(
            styles[id.0].taffy_style.flex_direction,
            taffy::FlexDirection::Column
        );
    }

    #[test]
    fn inline_inherited_sets_bit() {
        let (tree, _) = parse_html_to_ir(r#"<span style="color:#0000ff"></span>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        let color_bit = yio_core::style::dynamic::inherited_bit("color").unwrap();
        assert!(
            styles[id.0].inherited_set.0 & color_bit != 0,
            "inline color must set inherited_set COLOR bit"
        );
    }

    #[test]
    fn inline_non_inherited_sets_no_bit() {
        let (tree, _) = parse_html_to_ir(r#"<div style="width:100px"></div>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(
            styles[id.0].inherited_set.0, 0,
            "non-inherited width must not set any inherited bit"
        );
    }

    #[test]
    fn inline_font_size_sets_bit() {
        let (tree, _) = parse_html_to_ir(r#"<div style="font-size:20px"></div>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        let fs_bit = yio_core::style::dynamic::inherited_bit("font-size").unwrap();
        assert_eq!(
            styles[id.0].inherited_set.0 & fs_bit,
            fs_bit,
            "inline font-size must set inherited_set FONT_SIZE bit"
        );
    }

    /// 浏览器 UA 样式表：button 默认 text-align: center（继承到 text 子节点）。
    /// Yio 无 UA 样式表概念——按 tag semantic 直接设默认。
    /// 修前根因：button 元素 text-align=Left（无 UA 表，回落 ResolvedStyle::default Left）
    /// → text 子节点继承 Left → "Buy" 字不居中。
    #[test]
    fn button_default_text_align_is_center() {
        let (tree, _) = parse_html_to_ir(r#"<button>Buy</button>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(
            styles[id.0].text_align,
            yio_core::style::resolved::TextAlign::Center,
            "button UA 默认 text-align: center"
        );
        // UA 默认视为"显式声明"，set INH_TEXT_ALIGN bit——防 propagate_inherited
        // 把父（卡片/列表项等）的 text-align 覆盖到 button。
        let ta_bit = yio_core::style::dynamic::inherited_bit("text-align").unwrap();
        assert_eq!(
            styles[id.0].inherited_set.0 & ta_bit,
            ta_bit,
            "button UA text-align 必须置 INH_TEXT_ALIGN bit 防 propagate 覆盖"
        );
    }

    /// 用户显式声明 text-align 覆盖 button UA default（CSS 级联优先级）。
    #[test]
    fn explicit_text_align_overrides_button_default() {
        let (tree, _) = parse_html_to_ir(r#"<button style="text-align:left">Buy</button>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(
            styles[id.0].text_align,
            yio_core::style::resolved::TextAlign::Left,
            "显式 text-align:left 覆盖 button UA center"
        );
    }

    /// 非 button 元素 text-align 保持 default Left（不应被误改）。
    #[test]
    fn non_button_keeps_default_text_align() {
        let (tree, _) = parse_html_to_ir(r#"<div>hi</div>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(
            styles[id.0].text_align,
            yio_core::style::resolved::TextAlign::Left,
            "div UA 无 text-align 默认（保持 Left）"
        );
    }

    /// Bug 续修：button UA 容器居中（justify-content + align-items = center）。
    /// Bug B 只修 text-align center（text 内部居中），未修容器居中，
    /// text 子作为 flex item 从 padding-left 起——core dump 实证 text.x=266 应 268.5。
    /// 非继承属性 → 无 INH bit，仅本节点生效，但每帧 rematch 从 base_style 重起，稳定。
    #[test]
    fn button_default_flex_centering() {
        let (tree, _) = parse_html_to_ir(r#"<button>Buy</button>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(
            styles[id.0].taffy_style.justify_content,
            Some(taffy::JustifyContent::CENTER),
            "button UA justify-content: center"
        );
        assert_eq!(
            styles[id.0].taffy_style.align_items,
            Some(taffy::AlignItems::CENTER),
            "button UA align-items: center"
        );
    }

    /// 用户显式 justify/align 覆盖 button UA center（CSS 级联优先级）。
    #[test]
    fn explicit_justify_align_overrides_button_default() {
        let (tree, _) = parse_html_to_ir(
            r#"<button style="justify-content:flex-start; align-items:flex-end">x</button>"#,
        );
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_eq!(
            styles[id.0].taffy_style.justify_content,
            Some(taffy::JustifyContent::FLEX_START),
            "显式 justify-content 覆盖 button UA center"
        );
        assert_eq!(
            styles[id.0].taffy_style.align_items,
            Some(taffy::AlignItems::FLEX_END),
            "显式 align-items 覆盖 button UA center"
        );
    }

    /// 非 button 元素不沾 button UA center（防误改）。
    #[test]
    fn non_button_keeps_default_justify_align() {
        let (tree, _) = parse_html_to_ir(r#"<div>hi</div>"#);
        let styles = resolve_for_test(&tree);
        let id = tree.roots[0];
        assert_ne!(
            styles[id.0].taffy_style.justify_content,
            Some(taffy::JustifyContent::CENTER),
            "div 不沾 button UA center"
        );
    }

    /// animation 行内声明：合法值不报诊断（语法校验通过，apply_decl 不存）。
    /// runtime 没实现 → fence 接受语法 + 静默不跑动画。
    #[test]
    fn inline_animation_valid_no_diagnostic() {
        let (tree, _) = parse_html_to_ir(r#"<div style="animation:fadeIn .4s both"></div>"#);
        let (_styles, diags) =
            resolve_inline_styles_with_diags(&tree, "test.html", &LineMap::new(""));
        assert!(diags.is_empty(), "合法 animation 值不应报诊断: {diags:?}");
    }

    /// animation 行内声明：非法值报诊断（语法错误非静默）。
    #[test]
    fn inline_animation_invalid_reports_diagnostic() {
        let (tree, _) = parse_html_to_ir(r#"<div style="animation:bogusKeyword"></div>"#);
        let (_styles, diags) =
            resolve_inline_styles_with_diags(&tree, "test.html", &LineMap::new(""));
        assert!(
            diags
                .iter()
                .any(|d| d.code == DiagnosticCode::FenceBadCssValue
                    && d.message.contains("animation")),
            "非法 animation 值应报 FenceBadCssValue: {diags:?}"
        );
    }

    /// 围栏外常见属性（box-sizing 等）error message 必须带替代方案引导，
    /// 帮作者改到正确写法而非只说「不在围栏」。
    #[test]
    fn box_sizing_error_guides_to_removal() {
        let r = crate::parse_template(r#"<div style="box-sizing:border-box"></div>"#, "t.html");
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == DiagnosticCode::FenceUnknownCssProp)
            .expect("should error");
        assert!(
            d.message.contains("content-box"),
            "msg should state the content-box contract (#116): {}",
            d.message
        );
        assert!(
            d.message.contains("subtract"),
            "msg should guide the padding subtraction: {}",
            d.message
        );
    }

    #[test]
    fn visibility_error_guides_to_display_none() {
        let r = crate::parse_template(r#"<div style="visibility:hidden"></div>"#, "t.html");
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.code == DiagnosticCode::FenceUnknownCssProp)
            .expect("should error");
        assert!(
            d.message.contains("display:none"),
            "msg should suggest display:none: {}",
            d.message
        );
    }

    /// z-index 合法入栏：整数值（含负）通过；auto/垃圾值报 FenceBadCssValue
    /// （apply_decl 宽松降 0，围栏不静默降级）。夹具带 position:relative——
    /// 非 static 声明位才是 z-index 的合法落点（#101 E1 门）。
    #[test]
    fn z_index_integer_accepted_and_auto_rejected() {
        let ok = crate::parse_template(
            r#"<div style="position:relative;z-index:5"></div><div style="position:relative;z-index:-3"></div>"#,
            "t.html",
        );
        assert!(
            ok.diagnostics.is_empty(),
            "integer z-index should pass: {:?}",
            ok.diagnostics
        );
        let bad = crate::parse_template(
            r#"<div style="position:relative;z-index:auto"></div>"#,
            "t.html",
        );
        assert!(
            bad.diagnostics.iter().any(
                |d| d.code == DiagnosticCode::FenceBadCssValue && d.message.contains("z-index")
            ),
            "z-index:auto should error: {:?}",
            bad.diagnostics
        );
    }

    /// animation-* 长划入栏：单值合法通过；逗号列表（简写专属）报错。
    #[test]
    fn animation_longhands_accepted_and_list_rejected() {
        let ok = crate::parse_template(
            r#"<div style="animation-name:fade; animation-duration:.4s; animation-delay:.1s; animation-timing-function:ease-in; animation-iteration-count:3; animation-direction:alternate; animation-fill-mode:forwards; animation-play-state:running"></div>"#,
            "t.html",
        );
        assert!(
            ok.diagnostics.is_empty(),
            "longhands should pass: {:?}",
            ok.diagnostics
        );
        let bad = crate::parse_template(
            r#"<div style="animation-duration:.4s, .8s"></div>"#,
            "t.html",
        );
        assert!(
            bad.diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::FenceBadCssValue),
            "comma list in longhand should error: {:?}",
            bad.diagnostics
        );
    }

    /// flex-wrap:wrap-reverse 不支持——schema 删值后必须报 FenceBadCssValue，
    /// 而非像 v1 那样静默降级成 nowrap。
    #[test]
    fn flex_wrap_reverse_rejected() {
        let r = crate::parse_template(r#"<div style="flex-wrap:wrap-reverse"></div>"#, "t.html");
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::FenceBadCssValue
                    && d.message.contains("wrap-reverse")),
            "wrap-reverse should error: {:?}",
            r.diagnostics
        );
    }

    /// #74 `<a>` UA 烙印：打包期 default 色 #0000EE + text-decoration:underline，
    /// 且烙 INH_COLOR bit（color 继承属性，防运行时 propagate 拿父值覆盖链接色）。
    #[test]
    fn link_ua_defaults_blue_and_underline() {
        let (tree, _) = parse_html_to_ir(r#"<div>看<a href="x">商店</a></div>"#);
        let styles = resolve_for_test(&tree);
        let a_idx = tree
            .nodes
            .iter()
            .position(|n| matches!(&n.kind, IrNodeKind::Element(e) if e.tag == "a"))
            .expect("a element");
        assert_eq!(styles[a_idx].color, [0.0, 0.0, 238.0 / 255.0, 1.0]);
        assert_eq!(styles[a_idx].text_decoration, TextDecoration::Underline);
        let color_bit = yio_core::style::dynamic::inherited_bit("color").unwrap();
        assert_eq!(
            styles[a_idx].inherited_set.0 & color_bit,
            color_bit,
            "UA 链接色须烙 INH_COLOR bit 防 propagate 覆盖"
        );
    }

    /// #74：作者 inline 声明覆盖 UA 烙印（tag 默认先应用、作者后应用即赢）。
    #[test]
    fn author_overrides_link_ua() {
        let r = crate::parse_template(
            r#"<div>看<a href="x" style="color:#ff0000; text-decoration:none">商店</a></div>"#,
            "t.html",
        );
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        let a_idx = r
            .tree
            .nodes
            .iter()
            .position(|n| matches!(&n.kind, IrNodeKind::Element(e) if e.tag == "a"))
            .expect("a element");
        assert_eq!(
            r.styles[a_idx].color,
            [1.0, 0.0, 0.0, 1.0],
            "作者色覆盖 UA 蓝"
        );
        assert_eq!(
            r.styles[a_idx].text_decoration,
            TextDecoration::None,
            "作者 text-decoration:none 覆盖 UA underline"
        );
    }

    /// #74 text-decoration 值集：underline/none 过、line-through 拒（既有
    /// FenceBadCssValue 格式）。
    #[test]
    fn text_decoration_value_domain() {
        let ok = crate::parse_template(
            r#"<div>看<a href="x" style="text-decoration:underline">商店</a> <span style="text-decoration:none">普通</span></div>"#,
            "t.html",
        );
        assert!(ok.diagnostics.is_empty(), "{:?}", ok.diagnostics);
        let bad = crate::parse_template(
            r#"<div><span style="text-decoration:line-through">划线</span></div>"#,
            "t.html",
        );
        assert!(
            bad.diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::FenceBadCssValue
                    && d.message.contains("line-through")),
            "line-through 应按值域外报错: {:?}",
            bad.diagnostics
        );
    }
}
