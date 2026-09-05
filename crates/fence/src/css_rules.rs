//! `<style>` 选择器解析 + 规则表产物（fence = 纯解析器）。
//!
//! 路径 c：手搓解析器，直产 core 的 ParsedSelector/Compound（fence 已依赖 core）。
//! 子集：class / tag / id / 后代组合（空格）/ 伪类（hover/active/disabled/focus/checked/
//! nth-child(An+B|odd|even|N)）/ 属性选择器（[attr] / [attr="val"]，仅 Exists + Eq）。
//! 越界（nth-of-type 等、+ ~ 组合子等）返 None，由调用方报错。
//!
//! @keyframes at-rule（「动画定义全在 CSS」终态）：fence 解析
//! `@keyframes <name> { <stop-selector> { decls } ... }` 产 `KeyframesRule`。stop 声明块内
//! 或块之间的 `/* @yio-hook name */` 注释解析为锚点（挂在前一个 stop 上，供 player
//! 播放到该 stop 时发事件）。pkg v30 起 core 有同形类型并序列化进 pkg.bin；fence → core
//! 的类型转换（declarations → AnimatableProps）由打包器 bridge 完成。
//!
//! @yio-hook 的特殊处理：`parse_style_block` 将合法锚点注释替换为不可见 marker，普通
//! CSS 注释仍被剥除；`parse_keyframes_rule` 消费 marker 并将锚点挂到对应 stop。
use crate::css_resolve::unsupported_hint;
use crate::diagnostic::{Diagnostic, DiagnosticCode, LineMap, SourceLocation};
use crate::schema::css::{find_css_prop, find_shorthand};
use yio_core::style::dynamic::{
    AttrOp, AttrSelector, Combinator, Compound, Declaration, DynamicRule, NthChildExpr,
    ParsedSelector, Specificity,
};

/// `@keyframes` 一条 stop 的选择器位置。CSS 标准：`from`=`0%`，`to`=`100%`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyframeStopSelector {
    From,
    To,
    /// 0..=100，CSS 允许小数百分比但本围栏子集只接受整数（showcase 用法覆盖）。
    Percent(u8),
}

/// `@keyframes` 内一条 stop：选择器位置 + 声明块 + 锚点（如 `from { opacity:0 }`）。
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframeStop {
    pub selector: KeyframeStopSelector,
    pub declarations: Vec<Declaration>,
    /// `/* @yio-hook name */` 锚点：写在 stop 块后/块内，挂在该 stop 上。
    /// player 播放到该 stop 的百分比时发事件。None = 无锚点。
    pub hook: Option<String>,
}

/// `@keyframes <name> { ... }` 整体规则。stops 按 source 顺序保留（runtime 按需插值）。
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframesRule {
    pub name: String,
    pub stops: Vec<KeyframeStop>,
}

/// 解析单条选择器串 → ParsedSelector（含 specificity）。越界返 None。
///
/// 子集：空格分隔的若干 compound（后代组合）；每个 compound =
/// tag? + (class/id/pseudo/attr)*。
/// 越界：Child `>`、相邻 `+`/`~`、逗号多选（逗号在 parse_style_block 预切分）→ None。
/// 注意 `+`/`-` 在 `:nth-child(...)` 括号内合法（An+B），组合子判定按括号深度排除。
pub fn parse_selector(raw: &str) -> Option<ParsedSelector> {
    parse_selector_with_reason(raw).ok()
}

/// [`parse_selector`] 的带原因版：越界时 Err 携带具体构造（报错点名元凶）。
pub fn parse_selector_with_reason(raw: &str) -> Result<ParsedSelector, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty selector".to_string());
    }
    // 越界字符快速判定：逗号 / + ~ 组合子不在本子集（属性选择器 `[...]` 已支持，见
    // parse_compound；`:nth-child(2n+1)` 的 `+` 在括号内合法，按深度排除）。
    // `>` 已入子集（#114 子代组合器）——见下方切分循环。
    if let Some(ch) = out_of_subset_combinator(raw) {
        return Err(format!(
            "combinator \"{ch}\" is outside the fence (only descendant \" \" and child \">\" combinators)"
        ));
    }

    let mut specificity_a = 0u32; // id 数
    let mut specificity_b = 0u32; // class + 伪类 + 属性 数
    let mut specificity_c = 0u32; // tag 数
    let mut compounds: Vec<Compound> = Vec::new();

    // 按括号深度切分 compound：空白分隔后代链；`>`（括号外）自成 token（哨兵），
    // 其后随 compound 标 Child。四种写法 `a>b` / `a > b` / `a >b` / `a> b` 同一处理
    // （CSS 组合子两侧空白可有可无）。
    // `split_whitespace` 会拆坏括号内空格（`:nth-child(2n + 1)` 的 `+` 两侧空格
    // 合法，CSS An+B 语法允许），故手写深度扫描。
    let mut parts: Vec<&str> = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    for (idx, ch) in raw.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            '>' if depth == 0 => {
                if idx > start {
                    parts.push(&raw[start..idx]);
                }
                parts.push(">");
                start = idx + 1;
            }
            _ if ch.is_whitespace() && depth == 0 => {
                if idx > start {
                    parts.push(&raw[start..idx]);
                }
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    if start < raw.len() {
        parts.push(&raw[start..]);
    }

    let mut pending_child = false;
    for part in parts {
        if part == ">" {
            if compounds.is_empty() {
                return Err("selector must not start with a \">\" combinator".to_string());
            }
            if pending_child {
                return Err("duplicate \">\" combinator (missing compound between)".to_string());
            }
            pending_child = true;
            continue;
        }
        let (c, a, b, cc) = parse_compound_detailed(part)?;
        specificity_a += a;
        specificity_b += b;
        specificity_c += cc;
        // comps[i].combinator 描述它与 comps[i-1] 的关系；首个 compound 的字段无前驱，
        // matcher 不读。`>` 后随的 compound 标 Child（#114：复合控件嵌套态样式作用域）。
        let mut c = c;
        c.combinator = if pending_child {
            Combinator::Child
        } else {
            Combinator::Descendant
        };
        pending_child = false;
        compounds.push(c);
    }
    if pending_child {
        return Err("selector must not end with a \">\" combinator".to_string());
    }

    if compounds.is_empty() {
        return Err("empty selector".to_string());
    }
    if compounds[..compounds.len() - 1]
        .iter()
        .any(|c| c.part.is_some())
    {
        return Err("::part(name) must be the final compound".to_string());
    }
    Ok(ParsedSelector {
        raw: raw.to_string(),
        compound: compounds,
        specificity: Specificity(specificity_a, specificity_b, specificity_c),
    })
}

/// 组合子越界扫描：括号外出现 `,` / `+` / `~` 即越界，返回首个越界字符。
/// `>` 已入子集（#114，切分循环处理）；`:nth-child(An+B)` 的参数里 `+`/`-` 是
/// 合法语法（如 `2n+1`），括号内不判。
fn out_of_subset_combinator(raw: &str) -> Option<char> {
    let mut depth: i32 = 0;
    for ch in raw.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' | '+' | '~' if depth == 0 => return Some(ch),
            _ => {}
        }
    }
    None
}

/// [`parse_compound_detailed`] 的文档见上：失败时 Err 携带具体越界构造（供
/// 「unsupported selector」报错点名元凶——笼统的整串不支持会让 AI 读者
/// 误判成相邻构造的锅，如把 `:not()` 的错归给同串的 `:hover`）。
fn parse_compound_detailed(part: &str) -> Result<(Compound, u32, u32, u32), String> {
    let mut c = Compound {
        tag: None,
        classes: Vec::new(),
        id: None,
        combinator: Combinator::Descendant,
        pseudo_hover: false,
        pseudo_active: false,
        pseudo_disabled: false,
        pseudo_focus: false,
        pseudo_nth_child: None,
        attrs: Vec::new(),
        part: None,
    };
    let mut a = 0u32;
    let mut b = 0u32;
    let mut cc = 0u32;
    let mut rest = part;
    let mut consumed_tag = false;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let (name, next) = take_ident(r);
            if name.is_empty() {
                return Err(format!("empty class name in \"{part}\""));
            }
            c.classes.push(name.to_string());
            b += 1;
            rest = next;
        } else if let Some(r) = rest.strip_prefix('#') {
            let (name, next) = take_ident(r);
            if name.is_empty() {
                return Err(format!("empty id name in \"{part}\""));
            }
            c.id = Some(name.to_string());
            a += 1;
            rest = next;
        } else if let Some(r) = rest.strip_prefix("::") {
            // ::part(name)（#57）：唯一进围栏的伪元素。compound 其余字段匹配组件 host、
            // part 匹配展开子树内目标（core match_element_with_state 的 part 臂）。
            // 网页同源约束：伪元素必须位于 compound 结尾，后面不可再缀简单选择器。
            let (name, next) = take_ident(r);
            if name != "part" {
                return Err(format!(
                    "pseudo-element \"::{name}\" is outside the fence (only ::part(name))"
                ));
            }
            let after = next
                .strip_prefix('(')
                .ok_or("::part requires an argument: ::part(name)")?;
            let close = after
                .find(')')
                .ok_or("::part requires an argument: ::part(name)")?;
            let (pname, pnext) = take_ident(&after[..close]);
            if pname.is_empty() || !pnext.trim_start().is_empty() {
                return Err("::part takes exactly one non-empty name: ::part(name)".to_string());
            }
            c.part = Some(pname.to_string());
            b += 1; // part 名按属性选择器级计（web specificity）
            cc += 1; // 伪元素本体按元素级计（web specificity）
            rest = &after[close + 1..];
            if !rest.is_empty() {
                return Err(
                    "::part(name) must end the compound — nothing may follow a pseudo-element"
                        .to_string(),
                );
            }
        } else if let Some(r) = rest.strip_prefix(':') {
            let (name, next) = take_ident(r);
            match name {
                "hover" => {
                    c.pseudo_hover = true;
                    rest = next;
                }
                "active" => {
                    c.pseudo_active = true;
                    rest = next;
                }
                "disabled" => {
                    c.pseudo_disabled = true;
                    rest = next;
                }
                "focus" => {
                    c.pseudo_focus = true;
                    rest = next;
                }
                "checked" => {
                    // core 的 Compound 无 pseudo_checked 字段：checked 是控件态，由控件束处理。
                    // 本轮仅计 specificity（b+=1 在下方统一加），不存状态门。
                    rest = next;
                }
                "nth-child" => {
                    // 参数化伪类：`:nth-child(An+B|odd|even|N)`。
                    // 解析括号内 An+B → NthChildExpr；语法越界（无括号/缺 `)`/坏参数）→ Err。
                    let after = next.strip_prefix('(').ok_or_else(|| {
                        "invalid :nth-child(...) argument (An+B | odd | even | N)".to_string()
                    })?;
                    let close = after.find(')').ok_or_else(|| {
                        "invalid :nth-child(...) argument (An+B | odd | even | N)".to_string()
                    })?;
                    let (a, b) = parse_nth_arg(&after[..close]).ok_or_else(|| {
                        "invalid :nth-child(...) argument (An+B | odd | even | N)".to_string()
                    })?;
                    c.pseudo_nth_child = Some(NthChildExpr { a, b });
                    rest = &after[close + 1..];
                }
                "" => {
                    return Err(
                        "pseudo-elements (\"::before\" etc.) are outside the fence".to_string()
                    )
                }
                other => {
                    return Err(format!(
                        "pseudo-class \":{other}\" is outside the fence \
                         (supported: :hover, :active, :focus, :disabled, :checked, :nth-child)"
                    ))
                }
            }
            b += 1; // 伪类算 class 级
        } else if let Some(r) = rest.strip_prefix('[') {
            // 属性选择器：[attr] / [attr="val"] / [attr=val]。仅 Eq + Exists；高阶运算符
            // (^= ~= $= *= |=) 不在围栏子集 → Err 点名运算符。
            let close = r
                .find(']')
                .ok_or_else(|| "attribute selector is missing \"]\"".to_string())?;
            let inner = r[..close].trim();
            let after = &r[close + 1..];
            let (name, op, value) = match inner.find('=') {
                Some(eq_pos) => {
                    let name_part = inner[..eq_pos].trim();
                    // 高阶属性运算符的修饰字符紧贴 = 前 → 围栏外，点名运算符。
                    if let Some(modifier) = name_part
                        .chars()
                        .last()
                        .filter(|ch| ['~', '^', '$', '*', '|'].contains(ch))
                    {
                        return Err(format!(
                            "attribute operator \"{modifier}=\" is outside the fence \
                             (only [attr] and [attr=\"value\"])"
                        ));
                    }
                    if name_part.is_empty() {
                        return Err(format!("empty attribute name in \"{part}\""));
                    }
                    let val = inner[eq_pos + 1..]
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'');
                    (
                        name_part.to_ascii_lowercase(),
                        AttrOp::Eq,
                        Some(val.to_string()),
                    )
                }
                None => {
                    if inner.is_empty() {
                        return Err(format!("empty attribute name in \"{part}\""));
                    }
                    (inner.to_ascii_lowercase(), AttrOp::Exists, None)
                }
            };
            c.attrs.push(AttrSelector { name, op, value });
            b += 1; // 属性选择器算 class 级
            rest = after;
        } else {
            // tag（必须出现在 compound 最前）
            if consumed_tag {
                return Err(format!(
                    "invalid token \"{rest}\" — a compound is tag + classes/ids/pseudos/attrs"
                ));
            }
            if rest.starts_with('*') {
                return Err("universal selector \"*\" is outside the fence".to_string());
            }
            let (name, next) = take_ident(rest);
            if name.is_empty() {
                return Err(format!("invalid token \"{rest}\""));
            }
            c.tag = Some(name.to_string());
            cc += 1;
            consumed_tag = true;
            rest = next;
        }
    }
    Ok((c, a, b, cc))
}

/// 解析 `:nth-child(...)` 参数 → (a, b)。
///
/// 语法：`odd`=`2n+1`、`even`=`2n`、纯整数 `N`=`0n+N`、`An+B`。
/// An+B 按正则 `^(\d*)n\s*([+-]\s*\d+)?$` 手搓解析（零正则依赖）：
/// A 缺省（`n`）= 1，B 缺省 = 0，B 必须带符号（`2n1` 非法）。
/// 参数大小写不敏感（CSS 关键字 ASCII 大小写不敏感）。
fn parse_nth_arg(arg: &str) -> Option<(i32, i32)> {
    let t = arg.trim();
    if t.eq_ignore_ascii_case("odd") {
        return Some((2, 1));
    }
    if t.eq_ignore_ascii_case("even") {
        return Some((2, 0));
    }
    // 纯整数 N（可带符号，如 `-3`/`+3` 合法但恒不命中，index ≥ 1）
    if let Ok(n) = t.parse::<i32>() {
        return Some((0, n));
    }
    // An+B：先找 `n`，其前为 A（缺省 = 1），其后为带符号 B
    let lower = t.to_ascii_lowercase();
    let n_pos = lower.find('n')?;
    let a_part = lower[..n_pos].trim();
    let a: i32 = if a_part.is_empty() {
        1
    } else {
        a_part.parse().ok()?
    };
    let b_rest = lower[n_pos + 1..].trim();
    let b: i32 = if b_rest.is_empty() {
        0
    } else {
        // B 必须带符号（如 `2n1` 非法）：± 前缀 + 数字，缺符号或空数字 → 整体拒绝
        let signed = b_rest
            .strip_prefix('+')
            .or_else(|| b_rest.strip_prefix('-'))
            .filter(|d| !d.trim().is_empty())?;
        let sign = if b_rest.starts_with('-') { -1 } else { 1 };
        sign * signed.trim().parse::<i32>().ok()?
    };
    Some((a, b))
}

/// 取一个标识符（字母/数字/`-`/`_`），返回 (标识符, 剩余)。
fn take_ident(s: &str) -> (&str, &str) {
    let end = s
        .find(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

/// 解析 `<style>` 文本 → (动态规则表, keyframes 规则表, 诊断)（打包期模式）。
///
/// 文法（子集）：
/// - 普通规则：`selector { decl_list }`，selector 为单选择器（不支持逗号）。
/// - At-rule：`@keyframes <name> { <stop>{decls} ... }`（嵌套大括号）→ KeyframesRule。
///   其他 `@xxx` at-rule 不在围栏子集，整块丢弃 + 诊断。
/// - `decl_list` = `prop: value;` 重复。CSS 注释 `/* ... */` 剥除。
/// - 越界 selector / at-rule → 丢弃 + 诊断；声明 prop 名不在 schema（find_css_prop/find_shorthand）
///   → 诊断（与 css_resolve 一致）。例外：`--*` 自定义属性放行（#11），值近乎自由。
/// - 含 `var()` 的值只做形状校验（终值运行时在 var 环境解析）；同块 custom prop
///   引用环发 warning（运行时该环上属性全 invalid）。
///
/// @keyframes 解析后产出 KeyframesRule；packer bridge 将它翻译并序列化进 pkg.bin v30。
pub fn parse_style_block(css: &str) -> (Vec<DynamicRule>, Vec<KeyframesRule>, Vec<Diagnostic>) {
    parse_style_block_named(css, "<style>")
}

/// 解析模式：打包期（环 warning + @keyframes 合法）vs 运行时注入（at-rule 全拒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseMode {
    /// `<style>` 块打包期解析。
    Pack,
    /// `UIContext.StyleSheet.Add` 运行时注入解析（#11）：at-rule 一律拒
    /// （含 @keyframes——运行时动画注入不在本通道），环不发 warning
    /// （运行时解析自会 invalid 回退）。
    Runtime,
}

/// [`parse_style_block`] 带来源文件标签：诊断的 file 字段指向 CSS 来源（内联
/// `<style>` 或外部 CSS 文件路径），让作者报错能落对文件。
pub fn parse_style_block_named(
    css: &str,
    source_file: &str,
) -> (Vec<DynamicRule>, Vec<KeyframesRule>, Vec<Diagnostic>) {
    parse_block(css, source_file, ParseMode::Pack)
}

/// 运行时 CSS 注入解析（`UIContext.StyleSheet.Add` 通道，#11）。
/// Ok = 规则集（可注入 scene）；Err = 首条 Error 诊断（携带行列，C# 抛 UIStyleException）。
/// 与打包期的差异见 [`ParseMode::Runtime`]。
pub fn parse_runtime_css(css: &str) -> Result<Vec<DynamicRule>, Diagnostic> {
    let (rules, _keyframes, diags) = parse_block(css, "<runtime-css>", ParseMode::Runtime);
    if let Some(err) = diags
        .iter()
        .find(|d| d.severity == crate::diagnostic::Severity::Error)
    {
        return Err(err.clone());
    }
    Ok(rules)
}

fn parse_block(
    css: &str,
    source_file: &str,
    mode: ParseMode,
) -> (Vec<DynamicRule>, Vec<KeyframesRule>, Vec<Diagnostic>) {
    let stripped = strip_comments(css);
    // 诊断定位用（粗略）：strip_comments 后 offset 已不对应原文，但行号近似可用。
    let line_map = LineMap::new(&stripped);
    let mut rules = Vec::new();
    let mut keyframes = Vec::new();
    let mut diagnostics = Vec::new();
    // 每条 custom prop 声明 (prop, value, 所在规则 loc)——块级环 warning 定位用。
    let mut custom_decl_locs: Vec<(String, String, SourceLocation)> = Vec::new();
    let mut pos = 0;
    while pos < stripped.len() {
        let Some(brace_open_rel) = stripped[pos..].find('{') else {
            break;
        };
        let brace_open = pos + brace_open_rel;
        let prelude = stripped[pos..brace_open].trim();
        let (prelude, _) = remove_hook_markers(prelude);
        let after_open = brace_open + 1;
        let sel_start = pos;

        if prelude.starts_with('@') {
            let loc = line_map.source_location(sel_start, source_file.to_string());
            if mode == ParseMode::Runtime {
                // Add() 通道 at-rule 全拒（含 @keyframes）：fail-loud，不静默跳过
                // （作者以为注入成功实际被丢 = 预览≠运行时静默差异）。
                let at_name = prelude
                    .trim_start_matches('@')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::FenceBadCssValue,
                    format!(
                        "at-rule @{at_name} is rejected by runtime stylesheet injection — \
                         StyleSheet.Add accepts selector rules only (no @keyframes / @media; \
                         declare them in the package CSS instead)"
                    ),
                    loc,
                ));
                // 消费掉整块（按深度找配平 `}`），继续解析后续规则。
                if let Some((_, end_pos)) = find_matching_brace(&stripped, after_open) {
                    pos = end_pos;
                } else {
                    break;
                }
                continue;
            }
            // 找匹配的 `}`（@keyframes 体含嵌套大括号，必须按深度匹配）
            let Some((body, end_pos)) = find_matching_brace(&stripped, after_open) else {
                break;
            };
            pos = end_pos;
            let at_body = body;
            let at_kw_str = prelude.trim_start_matches('@').trim_start();
            let (at_name, at_rest) = split_at_keyword(at_kw_str);
            match at_name.as_str() {
                "keyframes" => match parse_keyframes_rule(&at_rest, at_body, &loc) {
                    Ok(kf) => {
                        // #10：layout 通道（width/height）端点校验——值必须显式长度域且
                        // 全 rule 同域（auto 不可动画、异域混合无法插值；transition 侧的
                        // 元素级扫描在 layout_transition_check，这里是 keyframes 停靠点侧）。
                        validate_keyframes_layout_endpoints(&kf, &loc, &mut diagnostics);
                        keyframes.push(kf);
                    }
                    Err(d) => diagnostics.push(d),
                },
                _ => diagnostics.push(Diagnostic::error(
                    DiagnosticCode::FenceBadCssValue,
                    format!("unsupported at-rule @{at_name} in {source_file}"),
                    loc,
                )),
            }
            continue;
        }

        // 普通选择器分支：用首 `}`（无嵌套）取 body
        let Some(brace_close_rel) = stripped[after_open..].find('}') else {
            break;
        };
        let body = &stripped[after_open..after_open + brace_close_rel];
        let (body, _) = remove_hook_markers(body);
        pos = after_open + brace_close_rel + 1;

        if prelude.is_empty() {
            continue;
        }
        // <style> 内无精确 per-token span —— 定位用选择器起点近似。
        let loc = line_map.source_location(sel_start, source_file.to_string());
        // 声明块只解析一次，逗号 selector list 的每段共享同一 declarations（clone）。
        let declarations = parse_declarations(&body, &loc, &mut diagnostics);
        if declarations.is_empty() {
            continue;
        }
        for d in &declarations {
            if d.prop.starts_with("--") {
                custom_decl_locs.push((d.prop.clone(), d.value.clone(), loc.clone()));
            }
        }
        // 逗号 selector list：`a, b, c { decls }` → 每段独立 parse_selector，共享声明块。
        // parse_selector 自身仍拒逗号（越界），由这里先 split，每段不再含逗号。
        for sel_raw in prelude.split(',') {
            let sel_raw = sel_raw.trim();
            if sel_raw.is_empty() {
                continue;
            }
            match parse_selector_with_reason(sel_raw) {
                Ok(selector) => rules.push(DynamicRule {
                    selector,
                    declarations: declarations.clone(),
                }),
                Err(reason) => diagnostics.push(Diagnostic::error(
                    DiagnosticCode::FenceBadCssValue,
                    format!("unsupported selector \"{sel_raw}\" in <style>: {reason}"),
                    loc.clone(),
                )),
            }
        }
    }
    // 同块 custom prop 引用环检测（#11 分层 fail-loud：打包期能静态查到的环发 warning；
    // 运行时该环上属性全 invalid，静默会让「为什么这条声明没生效」无从查起）。
    // 定位取环上首个声明成员所在规则的 loc。运行时模式不发（解析自会 invalid 回退）。
    if mode == ParseMode::Pack
        && custom_decl_locs
            .iter()
            .any(|(p, _, _): &(String, String, SourceLocation)| p.starts_with("--"))
    {
        for msg in crate::var_check::custom_prop_cycle_warnings(
            custom_decl_locs
                .iter()
                .map(|(p, v, _)| (p.as_str(), v.as_str())),
        ) {
            // 定位：环上第一个在本块声明的名字 → 其首条声明的规则 loc。
            let first_name = msg
                .split("var(")
                .nth(1)
                .and_then(|rest| rest.split(')').next())
                .unwrap_or("");
            let loc = custom_decl_locs
                .iter()
                .find(|(p, _, _)| p == first_name)
                .map(|(_, _, l)| l.clone())
                .unwrap_or_else(|| line_map.source_location(0, source_file.to_string()));
            diagnostics.push(Diagnostic::warning(
                DiagnosticCode::FenceCustomPropCycle,
                msg,
                loc,
            ));
        }
    }
    (rules, keyframes, diagnostics)
}

/// 在 `s[start..]` 中找与（已消费的）`{` 匹配的 `}`。返回 (body_slice, position_after_close)。
///
/// 用于 @keyframes 这类含嵌套大括号的 at-rule body：朴素 `find('}')` 会停在第一个内层 `}`，
/// 切错 body。按深度计数：`{` +1 / `}` -1，归 0 即匹配。
fn find_matching_brace(s: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 1;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[start..i], i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// 把 `@<keyword>` 后的首标识符分出来，剩下作为 prelude 余部（trim 后）。
/// `keyframes charge` → (`keyframes`, `charge`)；`media screen` → (`media`, `screen`)。
fn split_at_keyword(s: &str) -> (String, String) {
    let s = s.trim();
    let end = s.find(|ch: char| ch.is_whitespace()).unwrap_or(s.len());
    (s[..end].to_string(), s[end..].trim().to_string())
}

/// 解析 `@keyframes <name> { <body> }` 的 body → KeyframesRule。
///
/// body 文法：`<stop-selector-list> { decl_list }` 重复，stop-selector-list = 逗号分隔的
/// `from` / `to` / `<N>%`。逗号多 stop（`0%,100%{...}`）展开为多个 KeyframeStop 共享同声明块。
/// 任一 stop-selector 非法 → 整个 @keyframes 块丢弃 + 诊断（CSS 严格失败模式）。
///
/// `strip_comments` 会把合法的 `/* @yio-hook name */` 替换成不可见 marker，避免普通
/// CSS 解析丢失锚点。本函数在 stop 前导（通常是上一个 stop 块之后）和声明块内部消费
/// marker：前导注释挂前一个 stop，声明块内注释挂当前 stop。这样既支持 brief 的
/// `from{...}/* @yio-hook start */ to{...}`，也支持更直观的 `from{/* @yio-hook start */ ...}`。
fn parse_keyframes_rule(
    name: &str,
    body: &str,
    loc: &SourceLocation,
) -> Result<KeyframesRule, Diagnostic> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Diagnostic::error(
            DiagnosticCode::FenceBadCssValue,
            "@keyframes 缺少 name",
            loc.clone(),
        ));
    }
    let mut stops: Vec<KeyframeStop> = Vec::new();
    let mut pending_hooks: Vec<String> = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        let Some(brace_open_rel) = body[pos..].find('{') else {
            break;
        };
        let brace_open = pos + brace_open_rel;
        let (stop_sel_clean, leading_hooks) = remove_hook_markers(&body[pos..brace_open]);
        if !leading_hooks.is_empty() {
            if let Some(previous) = stops.last_mut() {
                previous.hook = leading_hooks.last().cloned();
            } else {
                // A hook before the first stop is most naturally associated with that stop.
                pending_hooks.extend(leading_hooks);
            }
        }
        let stop_sel_raw = stop_sel_clean.trim();
        let after_open = brace_open + 1;
        let Some((inner, end_pos)) = find_matching_brace(body, after_open) else {
            break;
        };
        pos = end_pos;
        if stop_sel_raw.is_empty() {
            continue;
        }
        // 逗号多 stop：`0%,100%` → 展开为 [Percent(0), Percent(100)]，每 stop 共享同 declarations
        let mut sel_parsed: Vec<KeyframeStopSelector> = Vec::new();
        for raw in stop_sel_raw.split(',') {
            let s = parse_stop_selector(raw.trim(), loc)?;
            sel_parsed.push(s);
        }
        let (inner_clean, inner_hooks) = remove_hook_markers(inner);
        let decls = parse_declarations(&inner_clean, loc, &mut Vec::new()); // stops 内 prop 名错误 tolerable
        let hook = inner_hooks.last().cloned().or_else(|| pending_hooks.pop());
        for sel in sel_parsed {
            stops.push(KeyframeStop {
                selector: sel,
                declarations: decls.clone(),
                hook: hook.clone(),
            });
        }
    }
    // A marker after the final `}` has no next selector to consume; attach it to the final stop.
    let (_, trailing_hooks) = remove_hook_markers(&body[pos..]);
    if let (Some(previous), Some(hook)) = (stops.last_mut(), trailing_hooks.last()) {
        previous.hook = Some(hook.clone());
    }
    if stops.is_empty() {
        return Err(Diagnostic::error(
            DiagnosticCode::FenceBadCssValue,
            format!("@keyframes {name} 缺少 stop（from/to/N% 块）"),
            loc.clone(),
        ));
    }
    Ok(KeyframesRule {
        name: name.to_string(),
        stops,
    })
}

/// 解析单个 stop 选择器：`from` / `to` / `<N>%`（0..=100 整数）。
fn parse_stop_selector(
    raw: &str,
    loc: &SourceLocation,
) -> Result<KeyframeStopSelector, Diagnostic> {
    match raw {
        "from" => Ok(KeyframeStopSelector::From),
        "to" => Ok(KeyframeStopSelector::To),
        _ => {
            let Some(num_str) = raw.strip_suffix('%') else {
                return Err(Diagnostic::error(
                    DiagnosticCode::FenceBadCssValue,
                    format!("@keyframes stop \"{}\" 不合法（应为 from / to / N%）", raw),
                    loc.clone(),
                ));
            };
            let pct: u32 = num_str.parse().map_err(|_| {
                Diagnostic::error(
                    DiagnosticCode::FenceBadCssValue,
                    format!("@keyframes stop \"{}\" 百分比非数字", raw),
                    loc.clone(),
                )
            })?;
            if pct > 100 {
                return Err(Diagnostic::error(
                    DiagnosticCode::FenceBadCssValue,
                    format!("@keyframes stop \"{}\" 超过 100%", raw),
                    loc.clone(),
                ));
            }
            Ok(KeyframeStopSelector::Percent(pct as u8))
        }
    }
}

const YIO_HOOK_MARKER_START: char = '\u{1}';
const YIO_HOOK_MARKER_END: char = '\u{2}';

/// 剥除 CSS 注释 `/* ... */`。UTF-8 安全：在 `&str` 上用 `find`（ASCII 针的偏移恒为 char 边界）。
/// 不能逐字节 `u8 as char`——会损坏非 ASCII（CJK font-family、content 文本）。
/// 合法 `@yio-hook` 注释保留为内部 marker，供 keyframes stop 解析；普通注释照常移除。
/// marker 在声明/选择器解析前由 `remove_hook_markers` 清掉。
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => {
                let comment = &rest[start + 2..start + 2 + end];
                if let Some(name) = parse_yio_hook_comment(comment) {
                    out.push(YIO_HOOK_MARKER_START);
                    out.push_str(name);
                    out.push(YIO_HOOK_MARKER_END);
                }
                rest = &rest[start + 2 + end + 2..];
            }
            None => {
                // An unclosed comment consumes the remainder, as before. It cannot contain a
                // complete `@yio-hook` comment and therefore must not produce a marker.
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parse exactly `@yio-hook <name>` from a comment body. The name is one non-whitespace
/// token (`\\S+`); a missing separator or trailing token is not a hook comment.
fn parse_yio_hook_comment(comment: &str) -> Option<&str> {
    let comment = comment.trim();
    let rest = comment.strip_prefix("@yio-hook")?;
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let mut tokens = rest.split_whitespace();
    let name = tokens.next()?;
    tokens.next().is_none().then_some(name)
}

/// Remove retained hook markers from a selector/declaration slice and collect their names in
/// source order. Markers only occur when they came from a syntactically closed CSS comment.
fn remove_hook_markers(s: &str) -> (String, Vec<String>) {
    let mut clean = String::with_capacity(s.len());
    let mut hooks = Vec::new();
    let mut rest = s;
    loop {
        let Some(start) = rest.find(YIO_HOOK_MARKER_START) else {
            clean.push_str(rest);
            break;
        };
        clean.push_str(&rest[..start]);
        let after_start = start + YIO_HOOK_MARKER_START.len_utf8();
        let Some(end_rel) = rest[after_start..].find(YIO_HOOK_MARKER_END) else {
            // Defensive: markers are generated as a pair, but preserve malformed text rather
            // than silently dropping source bytes if this helper is reused later.
            clean.push_str(&rest[start..]);
            break;
        };
        let name = &rest[after_start..after_start + end_rel];
        if !name.is_empty() && !name.chars().any(char::is_whitespace) {
            hooks.push(name.to_string());
        }
        rest = &rest[after_start + end_rel + YIO_HOOK_MARKER_END.len_utf8()..];
    }
    (clean, hooks)
}

/// 解析声明块体 → Vec<Declaration>。prop 名校验同 css_resolve（find_css_prop/find_shorthand）。
/// `loc` = 本规则块的近似 SourceLocation（diagnostic 定位用）。
fn parse_declarations(
    body: &str,
    loc: &SourceLocation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Declaration> {
    let mut decls = Vec::new();
    for raw_decl in body.split(';') {
        let raw_decl = raw_decl.trim();
        if raw_decl.is_empty() {
            continue;
        }
        let Some((prop, value)) = raw_decl.split_once(':') else {
            continue;
        };
        let prop = prop.trim();
        let value = value.trim();
        if prop.is_empty() || value.is_empty() {
            continue;
        }
        // `--*` 自定义属性（#11）：prop 白名单放行，值近乎自由（CSS 规范行为：custom
        // prop 值不做关键字校验，坏值在 var() 消费端暴露为 invalid），只校验 var() 形状。
        if crate::var_check::is_custom_prop(prop) {
            if let Some(msg) = crate::var_check::var_shape_error(value) {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::FenceBadCssValue,
                    msg,
                    loc.clone(),
                ));
            } else {
                decls.push(Declaration {
                    prop: prop.to_string(),
                    value: value.to_string(),
                });
            }
            continue;
        }
        // 含 var() 的普通属性值（#11）：终值运行时在 var 环境解析（SetVar 可注入目标），
        // 打包期字面校验整条跳过、只做形状校验——与下面的 literal 门互斥。
        if crate::var_check::value_has_var(value) {
            if let Some(msg) = crate::var_check::var_shape_error(value) {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::FenceBadCssValue,
                    msg,
                    loc.clone(),
                ));
            } else {
                decls.push(Declaration {
                    prop: prop.to_string(),
                    value: value.to_string(),
                });
            }
            continue;
        }
        if find_css_prop(prop).is_none() && find_shorthand(prop).is_none() {
            let hint = unsupported_hint(prop)
                .unwrap_or("not supported by fence — remove or replace with a supported property.");
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::FenceUnknownCssProp,
                format!("CSS property \"{}\": {}", prop, hint),
                loc.clone(),
            ));
            continue;
        }
        // 共享值域门：宽松吞值通道（颜色/overflow/filter/transform）+ Keyword 域 +
        // display:inline 语义警告。此前 `<style>` 规则值不校验——命名色 / overflow:clip /
        // filter:blur 等在类规则里静默吞值（与 inline 路径不同门），此处统一。
        if let Some(msg) = crate::value_check::value_error(prop, value) {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::FenceBadCssValue,
                msg,
                loc.clone(),
            ));
            continue;
        }
        if let Some(msg) = crate::value_check::keyword_error(prop, value) {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::FenceBadCssValue,
                msg,
                loc.clone(),
            ));
            continue;
        }
        if let Some(note) = crate::value_check::display_inline_warning(value) {
            diagnostics.push(Diagnostic::warning(
                DiagnosticCode::FenceDisplayInline,
                format!("CSS property \"display\": {note}"),
                loc.clone(),
            ));
        }
        if prop == "transition" {
            for msg in crate::value_check::transition_warnings(value) {
                diagnostics.push(Diagnostic::warning(
                    DiagnosticCode::FenceTransitionUnsupportedProp,
                    msg,
                    loc.clone(),
                ));
            }
        }
        // 渐变值探针：`<style>` 规则的值不逐条校验（非关键字值运行时 apply_decl 才
        // 解析），但渐变子集是结构化值（stop 数上限 / radial 配置段语法），坏值静默
        // 到运行时丢背景太晚——打包期用 core `parse_gradient`（与运行时同一真相源）
        // 探测，失败即报。任何 `*-gradient(` 前缀值都必须过探针（conic / repeating-*
        // 是 parse_gradient 不认的围栏外形态，返 None 即报）；url()/纯色走原宽松路径。
        if (prop == "background-image" || prop == "background")
            && value.contains("-gradient(")
            && yio_core::style::mapping::parse_gradient(value).is_none()
        {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::FenceBadCssValue,
                format!(
                    "value \"{}\" is not valid for CSS property \"{}\" (gradient subset: \
                     `linear-gradient` / `radial-gradient` only, up to 8 stops — the \
                     `background-image` row of `css-reference.md` in the scaffolded \
                     yio-editor skill lists the accepted forms)",
                    value, prop
                ),
                loc.clone(),
            ));
            continue;
        }
        // 纯整数域属性（z-index/order）严格校验：与 css_resolve inline 路径同门——
        // apply_decl 宽松降级 0，围栏不静默降级（font-weight 等 Integer parser 属性
        // 接受关键字，不在此列）。
        if matches!(prop, "z-index" | "order") && value.parse::<i32>().is_err() {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::FenceBadCssValue,
                format!(
                    "value \"{}\" is not valid for CSS property \"{}\" (integer required)",
                    value, prop
                ),
                loc.clone(),
            ));
            continue;
        }
        decls.push(Declaration {
            prop: prop.to_string(),
            value: value.to_string(),
        });
    }
    decls
}

/// @keyframes 停靠点内 layout 通道（width/height）的端点校验（#10）：
/// - 值必须是显式长度域（`<n>px|%|vw|vh|vmin|vmax`，裸数字按 px）——auto / calc /
///   keyword 是不可动画端点，error（浏览器会平滑过渡到 auto，先验分歧必须响亮拒绝）；
/// - 同一 rule 内所有停靠点的同属性域必须一致（异域无法插值，error）。
///
/// 诊断定位用 rule 起点（`<style>` 内无 per-stop 精确 span，同选择器近似先例）。
fn validate_keyframes_layout_endpoints(
    kf: &KeyframesRule,
    loc: &crate::diagnostic::SourceLocation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    use crate::layout_transition_check::endpoint_domain_of;
    use crate::layout_transition_check::EndpointDomain;
    for prop in ["width", "height"] {
        // (stop 序号, 值, 域) —— 序号进报文，作者好定位停靠点。
        let mut seen: Vec<(usize, String, EndpointDomain)> = Vec::new();
        for (i, stop) in kf.stops.iter().enumerate() {
            for d in &stop.declarations {
                if d.prop == prop {
                    seen.push((i, d.value.clone(), endpoint_domain_of(&d.value)));
                }
            }
        }
        if seen.is_empty() {
            continue;
        }
        let mut domains: Vec<EndpointDomain> = seen.iter().map(|(_, _, d)| *d).collect();
        domains.sort_by_key(|d| d.label());
        domains.dedup();
        let values = seen
            .iter()
            .map(|(i, v, d)| format!("stop#{i} `{v}` ({})", d.label()))
            .collect::<Vec<_>>()
            .join(", ");
        if domains.len() > 1 {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::FenceLayoutTransitionEndpoint,
                format!(
                    "@keyframes {name}: {prop} endpoints mix domains — layout animation \
                     endpoints must stay in ONE domain (px↔px, %↔%, vw↔vw). Endpoints: \
                     {values}. Mixed-domain endpoints jump instantly instead of animating, \
                     so the fence rejects them.",
                    name = kf.name
                ),
                loc.clone(),
            ));
        } else if matches!(domains[0], EndpointDomain::Auto | EndpointDomain::Other) {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::FenceLayoutTransitionEndpoint,
                format!(
                    "@keyframes {name}: {prop} endpoint {} is not animatable — use explicit \
                     px / % / vw / vh / vmin / vmax values. `auto` and non-length values \
                     jump instantly instead of animating (browsers animate them), so the \
                     fence rejects them. Endpoints: {values}.",
                    domains[0].label(),
                    name = kf.name
                ),
                loc.clone(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(raw: &str) -> ParsedSelector {
        parse_selector(raw).unwrap_or_else(|| panic!("parse_selector({raw:?}) 返回 None"))
    }

    /// ::part(name) 解析（#57）：compound 其余字段归 host 匹配、part 归目标；
    /// specificity 按 web（part 名属性级 + 伪元素元素级）。
    #[test]
    fn part_selector_parses_with_host_fields_and_specificity() {
        let s = parse_selector(".card::part(title)").unwrap();
        assert_eq!(s.compound.len(), 1);
        assert_eq!(s.compound[0].classes, vec!["card".to_string()]);
        assert_eq!(s.compound[0].part.as_deref(), Some("title"));
        assert_eq!(s.specificity, Specificity(0, 2, 1));

        // 多 compound 前缀：part 只允许最后一个 compound
        let s2 = parse_selector(".list .card::part(t)").unwrap();
        assert_eq!(s2.compound.len(), 2);
        assert_eq!(s2.compound[1].part.as_deref(), Some("t"));
        assert!(s2.compound[0].part.is_none());

        // 裸 ::part(name)（无 host 定位条件）合法
        let s3 = parse_selector("::part(title)").unwrap();
        assert_eq!(s3.compound[0].part.as_deref(), Some("title"));

        // host 侧伪类正常共存
        let s4 = parse_selector(".card:hover::part(title)").unwrap();
        assert!(s4.compound[0].pseudo_hover);
        assert_eq!(s4.compound[0].part.as_deref(), Some("title"));
    }

    /// ::part 越界形态全拒：伪元素后缀、跨 compound、空/多参数、其他伪元素、无括号。
    #[test]
    fn part_selector_rejects_malformed_forms() {
        assert!(
            parse_selector(".a::part(x).b").is_none(),
            "伪元素后不可再缀"
        );
        assert!(
            parse_selector(".a::part(x) .b").is_none(),
            "part 必须是最后一个 compound"
        );
        assert!(parse_selector("::part()").is_none(), "空 name");
        assert!(parse_selector("::part(a b)").is_none(), "多参数");
        assert!(parse_selector(".a::part").is_none(), "无括号");
        assert!(parse_selector("::before").is_none(), "其他伪元素仍越界");
        assert!(parse_selector(".a::after").is_none());
        assert!(parse_selector(".a::partX(x)").is_none(), "非 part 伪元素");
    }

    #[test]
    fn selector_errors_name_the_culprit() {
        // 报错点名元凶：整串笼统「unsupported selector」会让 AI 读者把
        // `:not()` 的错归给同串的 `:hover`。
        let cases: &[(&str, &str)] = &[
            (".btn:hover:not(.x)", "pseudo-class \":not\""),
            (".btn::before", "only ::part(name)"),
            ("*:hover", "universal selector \"*\""),
            (".a + .b", "combinator \"+\""),
            ("[data-x^=\"y\"]", "attribute operator \"^=\""),
            (".a:nth-child(bad)", ":nth-child"),
        ];
        for (raw, expected) in cases {
            let err = match parse_selector_with_reason(raw) {
                Err(e) => e,
                Ok(_) => panic!("{raw:?} 应失败"),
            };
            assert!(
                err.contains(expected),
                "{raw:?} 报错应点名 {expected:?}，实得 {err:?}"
            );
        }
    }

    #[test]
    fn hover_alone_parses() {
        // 对照：`:hover` 本体在围栏内，与伪元素/未知伪类/通配区分。
        let s = spec(".btn:hover");
        assert!(s.compound[0].pseudo_hover);
    }

    #[test]
    fn class_selector() {
        let s = spec(".foo");
        assert_eq!(s.compound.len(), 1);
        assert_eq!(s.compound[0].classes, vec!["foo".to_string()]);
        // specificity (id, class, tag) = (0,1,0)
        assert_eq!(s.specificity.0, 0);
        assert_eq!(s.specificity.1, 1);
        assert_eq!(s.specificity.2, 0);
    }

    #[test]
    fn tag_selector() {
        let s = spec("div");
        assert_eq!(s.compound[0].tag.as_deref(), Some("div"));
        assert_eq!(
            s.specificity,
            yio_core::style::dynamic::Specificity(0, 0, 1)
        );
    }

    #[test]
    fn id_selector() {
        let s = spec("#bar");
        assert_eq!(s.compound[0].id.as_deref(), Some("bar"));
        assert_eq!(s.specificity.1, 0);
        assert_eq!(s.specificity.0, 1);
    }

    #[test]
    fn compound_class_tag_id() {
        // div.foo#bar → (id=1, class=1, tag=1)
        let s = spec("div.foo#bar");
        assert_eq!(s.compound[0].tag.as_deref(), Some("div"));
        assert_eq!(s.compound[0].classes, vec!["foo".to_string()]);
        assert_eq!(s.compound[0].id.as_deref(), Some("bar"));
        assert_eq!(
            s.specificity,
            yio_core::style::dynamic::Specificity(1, 1, 1)
        );
    }

    #[test]
    fn descendant_combinator() {
        // .a .b → 两个 compound，后者 combinator = Descendant
        let s = spec(".a .b");
        assert_eq!(s.compound.len(), 2);
        assert_eq!(s.compound[1].combinator, Combinator::Descendant);
        assert_eq!(s.specificity.1, 2); // 两个 class
    }

    #[test]
    fn pseudo_class_sets_flag_and_specificity() {
        let s = spec(".btn:hover");
        assert!(s.compound[0].pseudo_hover);
        // 伪类算 class 级 specificity → (0, 2, 0)
        assert_eq!(s.specificity.1, 2);
    }

    #[test]
    fn out_of_subset_returns_none() {
        // 属性选择器现已支持（[attr]/[attr="val"]）；逗号在 parse_style_block 预切分，
        // parse_selector 自身仍拒；+ ~ 组合子仍越界（`+` 在 :nth-child 括号内合法）。
        // `>` 已入子集（child_combinator 系列用例）。
        assert!(parse_selector(r#"[type="text"]"#).is_some());
        assert!(parse_selector(".a, .b").is_none());
        assert!(parse_selector(".a + .b").is_none());
        assert!(parse_selector(":nth-of-type(2)").is_none()); // 其他 nth-* 不在子集
                                                              // 属性选择器越界形态须显式拒（防静默降级：否则坏 selector 会被默默吞，
                                                              // 用户 CSS 静默失效）。仅支持 = / 裸 [attr]；修饰符操作符 / 空名 / 缺 ] 均拒。
        assert!(parse_selector("[a^=b]").is_none());
        assert!(parse_selector("[a~=b]").is_none());
        assert!(parse_selector("[=x]").is_none());
        assert!(parse_selector("[]").is_none());
        assert!(parse_selector("[a=b").is_none());
    }

    #[test]
    fn child_combinator() {
        // #114：`>` 入子集。`.a > .b` → 两 compound，后者 combinator = Child。
        let s = spec(".a > .b");
        assert_eq!(s.compound.len(), 2);
        assert_eq!(s.compound[0].classes, vec!["a".to_string()]);
        assert_eq!(s.compound[1].combinator, Combinator::Child);
        assert_eq!(s.compound[1].classes, vec!["b".to_string()]);
        assert_eq!(s.specificity.1, 2); // 组合子不加 specificity
    }

    #[test]
    fn child_combinator_whitespace_variants() {
        // CSS 组合子两侧空白可有可无，四种写法同解析。
        for raw in [".a>.b", ".a >.b", ".a> .b", ".a  >  .b"] {
            let s = parse_selector(raw).unwrap_or_else(|| panic!("{raw} should parse"));
            assert_eq!(s.compound.len(), 2, "{raw}");
            assert_eq!(s.compound[1].combinator, Combinator::Child, "{raw}");
        }
    }

    #[test]
    fn child_combinator_chain() {
        // .a > .b .c：中段 Child、末段 Descendant（混合链）。
        let s = spec(".a > .b .c");
        assert_eq!(s.compound.len(), 3);
        assert_eq!(s.compound[1].combinator, Combinator::Child);
        assert_eq!(s.compound[2].combinator, Combinator::Descendant);
    }

    #[test]
    fn child_combinator_malformed_rejected() {
        // 起始/结尾/连续 `>` 均显式拒（防静默吞——坏 selector 静默失效同属性选择器先例）。
        assert!(parse_selector("> .a").is_none());
        assert!(parse_selector(".a >").is_none());
        assert!(parse_selector(".a > > .b").is_none());
    }

    #[test]
    fn child_combinator_does_not_leak_inside_parens() {
        // `:nth-child(...)` 参数内的 `+`/`-` 合法（既有行为）；本用例锁「括号内
        // 不受组合子扫描影响」在 `>` 放行后仍成立——伪类参数解析路径零改动。
        let s = spec(".a:nth-child(2n+1)");
        assert!(s.compound[0].pseudo_nth_child.is_some());
    }

    #[test]
    fn child_combinator_specificity_unchanged() {
        // 组合子不参与 specificity：`.a > .b` 与 `.a .b` 同为 (0,2,0)。
        assert_eq!(spec(".a > .b").specificity, spec(".a .b").specificity);
    }

    #[test]
    fn parse_attr_selector_eq() {
        let s = parse_selector(r#"input[type="text"]"#).unwrap();
        assert_eq!(s.compound[0].tag.as_deref(), Some("input"));
        assert_eq!(s.compound[0].attrs.len(), 1);
        let a = &s.compound[0].attrs[0];
        assert_eq!(a.name, "type");
        assert_eq!(a.op, AttrOp::Eq);
        assert_eq!(a.value.as_deref(), Some("text"));
        // 属性选择器算 class 级 specificity → (id=0, class+attr=1, tag=1)
        assert_eq!(s.specificity, Specificity(0, 1, 1));
    }

    #[test]
    fn parse_attr_selector_unquoted_and_exists() {
        assert_eq!(
            parse_selector(r#"input[type=password]"#).unwrap().compound[0].attrs[0]
                .value
                .as_deref(),
            Some("password")
        );
        // [attr] 存在形式
        let s = parse_selector(r#"[disabled]"#).unwrap();
        assert_eq!(s.compound[0].attrs[0].op, AttrOp::Exists);
        assert!(s.compound[0].attrs[0].value.is_none());
    }

    use yio_core::style::dynamic::Declaration;

    #[test]
    fn parse_style_block_basic() {
        let css = ".foo { color: #ff0000; font-size: 24px }\ndiv.bar { width: 100px }";
        let (rules, _kf, diags) = parse_style_block(css);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector.raw, ".foo");
        assert_eq!(rules[0].declarations.len(), 2);
        assert_eq!(
            rules[0].declarations[0],
            Declaration {
                prop: "color".into(),
                value: "#ff0000".into()
            }
        );
        assert_eq!(rules[0].declarations[1].prop, "font-size");
        assert_eq!(rules[1].selector.raw, "div.bar");
        assert_eq!(rules[1].declarations[0].prop, "width");
    }

    #[test]
    fn parse_style_block_skips_unparseable_selector() {
        // .a + .b 越界 → 该规则进 diagnostic，其他规则照常（`>` 已入子集，改用 `+`）
        let (rules, _kf, diags) =
            parse_style_block(".a + .b { color: #ff0000 }\n.ok { color: #0000ff }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.raw, ".ok");
        assert!(
            diags.iter().any(|d| d.message.contains(".a + .b")),
            "越界选择器应报错: {diags:?}"
        );
    }

    #[test]
    fn parse_comma_selector_list_expands_to_shared_declarations() {
        // 逗号 selector list：`a, b, c { decls }` → 3 条 DynamicRule 共享同一声明块。
        // 用纯 tag 选择器隔离逗号展开机制本身。
        let (rules, _, diags) = parse_style_block("input, select, textarea { color: #ff0000 }");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(rules.len(), 3, "逗号 list 展开为 3 条规则");
        assert_eq!(rules[0].declarations, rules[1].declarations);
        assert_eq!(rules[1].declarations, rules[2].declarations);
    }

    #[test]
    fn parse_style_block_ignores_comments() {
        let (rules, _kf, _diags) = parse_style_block("/* c */ .x { color: #ff0000 } /* tail */");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.raw, ".x");
    }

    #[test]
    fn parse_style_block_preserves_cjk_and_strips_comment() {
        // UTF-8 safety: 注释 + CJK font-family 都不能被 strip_comments 损坏
        // （旧的 bytes[i] as char 字节循环会破坏多字节序列）。
        let (rules, _kf, diags) = parse_style_block("/* 注释 */ .x { font-family: \"微软雅黑\" }");
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.raw, ".x");
        assert_eq!(rules[0].declarations.len(), 1);
        assert_eq!(rules[0].declarations[0].prop, "font-family");
        assert_eq!(rules[0].declarations[0].value, "\"微软雅黑\"");
    }

    #[test]
    fn parse_style_block_keyframes_from_to() {
        // character.html 用法
        let css = "@keyframes charge { from{filter:brightness(.7)} to{filter:brightness(1)} }";
        let (_rules, keyframes, diags) = parse_style_block(css);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(keyframes.len(), 1, "应解析出 1 个 @keyframes");
        let kf = &keyframes[0];
        assert_eq!(kf.name, "charge");
        assert_eq!(kf.stops.len(), 2, "from + to → 2 stops");
        assert_eq!(kf.stops[0].selector, KeyframeStopSelector::From);
        assert_eq!(kf.stops[1].selector, KeyframeStopSelector::To);
        assert_eq!(kf.stops[0].declarations.len(), 1);
        assert_eq!(kf.stops[0].declarations[0].prop, "filter");
    }

    #[test]
    fn parse_style_block_keyframes_multi_percent_stops() {
        // 多 stop 百分比（home/lab 类）
        let css = "@keyframes fade { 0%{opacity:0} 50%{opacity:.5} 100%{opacity:1} }";
        let (_rules, keyframes, diags) = parse_style_block(css);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(keyframes.len(), 1);
        let kf = &keyframes[0];
        assert_eq!(kf.name, "fade");
        assert_eq!(kf.stops.len(), 3);
        assert_eq!(kf.stops[0].selector, KeyframeStopSelector::Percent(0));
        assert_eq!(kf.stops[1].selector, KeyframeStopSelector::Percent(50));
        assert_eq!(kf.stops[2].selector, KeyframeStopSelector::Percent(100));
    }

    #[test]
    fn parse_style_block_keyframes_comma_stop_selector_expands() {
        // mail.html 用法：`0%,100%{opacity:1} 50%{opacity:.4}` → 展开 3 stops
        let css = "@keyframes breathe { 0%,100%{opacity:1} 50%{opacity:.4} }";
        let (_rules, keyframes, diags) = parse_style_block(css);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(keyframes.len(), 1);
        let kf = &keyframes[0];
        assert_eq!(kf.name, "breathe");
        assert_eq!(kf.stops.len(), 3, "0%,100% 展开为 2 + 1 = 3 stops");
        // 按 source 顺序：Percent(0), Percent(100), Percent(50)
        assert_eq!(kf.stops[0].selector, KeyframeStopSelector::Percent(0));
        assert_eq!(kf.stops[1].selector, KeyframeStopSelector::Percent(100));
        assert_eq!(kf.stops[2].selector, KeyframeStopSelector::Percent(50));
        // 0% 与 100% 共享同 declarations（来自同一块）
        assert_eq!(kf.stops[0].declarations, kf.stops[1].declarations);
    }

    #[test]
    fn parse_style_block_keyframes_with_other_rules_interleaved() {
        // home.html 用法：@keyframes 块 + 后续 selector 规则混合
        let css =
            "@keyframes fadeIn { from{opacity:0} to{opacity:1} }\n.nav-card { color:#ff0000 }";
        let (rules, keyframes, diags) = parse_style_block(css);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(keyframes.len(), 1, "@keyframes 解析");
        assert_eq!(keyframes[0].name, "fadeIn");
        assert_eq!(rules.len(), 1, "普通 selector 规则照常解析");
        assert_eq!(rules[0].selector.raw, ".nav-card");
    }

    #[test]
    fn hook_comment_outside_keyframes_is_inert_in_declarations() {
        let (rules, keyframes, diags) =
            parse_style_block(".card { /* @yio-hook x */ color:#ff0000 }");
        assert!(keyframes.is_empty());
        assert!(
            diags.is_empty(),
            "normal-rule hook comment must not create diagnostics: {diags:?}"
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.raw, ".card");
        assert_eq!(rules[0].declarations.len(), 1);
        assert_eq!(rules[0].declarations[0].prop, "color");
        assert_eq!(rules[0].declarations[0].value, "#ff0000");
    }

    #[test]
    fn hook_comment_before_normal_rule_is_inert_in_selector() {
        let (rules, keyframes, diags) =
            parse_style_block("/* @yio-hook x */\n.card{color:#ff0000}");
        assert!(keyframes.is_empty());
        assert!(
            diags.is_empty(),
            "leading normal-rule hook comment must not create diagnostics: {diags:?}"
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.raw, ".card");
        assert_eq!(rules[0].declarations[0].prop, "color");
    }

    #[test]
    fn parse_style_block_keyframes_hook_after_stop_attaches_to_previous_stop() {
        let css = "@keyframes slideIn{from{opacity:0}/* @yio-hook start */ to{opacity:1}}";
        let (_rules, keyframes, diags) = parse_style_block(css);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(keyframes.len(), 1);
        assert_eq!(keyframes[0].stops[0].hook.as_deref(), Some("start"));
        assert_eq!(keyframes[0].stops[1].hook, None);
    }

    #[test]
    fn parse_style_block_keyframes_hook_inside_stop_attaches_to_current_stop() {
        let css = "@keyframes slideIn{from{/* @yio-hook start */ opacity:0}to{opacity:1}}";
        let (_rules, keyframes, diags) = parse_style_block(css);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(keyframes[0].stops[0].hook.as_deref(), Some("start"));
        assert_eq!(keyframes[0].stops[0].declarations[0].prop, "opacity");
    }

    #[test]
    fn parse_style_block_ignores_non_hook_comments() {
        let css = "@keyframes slideIn{from{opacity:0}/* ordinary */to{opacity:1}}";
        let (_rules, keyframes, diags) = parse_style_block(css);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(keyframes[0].stops[0].hook, None);
        assert_eq!(keyframes[0].stops[1].hook, None);
    }

    #[test]
    fn parse_style_block_keyframes_single_to_stop() {
        // lab.html 用法：只有 to stop（CSS 合法：from 隐式 = 当前状态）
        let css = "@keyframes shimmer { to { background-position:200% center; } }";
        let (_rules, keyframes, diags) = parse_style_block(css);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(keyframes.len(), 1);
        assert_eq!(keyframes[0].stops.len(), 1);
        assert_eq!(keyframes[0].stops[0].selector, KeyframeStopSelector::To);
    }

    #[test]
    fn parse_style_block_unknown_at_rule_errors() {
        // @media / @font-face 不在围栏子集 → diagnostic
        let (_rules, _kf, diags) = parse_style_block("@media screen { .x { color:#ff0000 } }");
        assert!(
            diags.iter().any(|d| d.message.contains("@media")),
            "未知 at-rule 应报错: {diags:?}"
        );
    }

    #[test]
    fn parse_style_block_keyframes_missing_name_errors() {
        let (_rules, _kf, diags) = parse_style_block("@keyframes { from{opacity:0} }");
        assert!(!diags.is_empty(), "无名 @keyframes 应报错");
    }

    #[test]
    fn parse_style_block_keyframes_over_100_pct_errors() {
        let (_rules, _kf, diags) = parse_style_block("@keyframes x { 150%{opacity:0} }");
        assert!(!diags.is_empty(), "百分比 > 100 应报错");
    }

    // ===== #11 custom props / var() / 运行时注入解析 =====

    #[test]
    fn parse_custom_prop_declarations_in_rules() {
        let (rules, _kf, diags) =
            parse_style_block(".theme { --accent: #ff0000; color: var(--accent) }");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations.len(), 2);
        assert_eq!(rules[0].declarations[0].prop, "--accent");
        assert_eq!(rules[0].declarations[1].value, "var(--accent)");
    }

    #[test]
    fn parse_var_bad_shape_errors() {
        // 名字缺 -- 前缀 / 括号不配平 → FenceBadCssValue
        let (_r, _k, diags) = parse_style_block(".x { color: var(accent) }");
        assert!(
            diags.iter().any(|d| d.message.contains("custom property")),
            "{diags:?}"
        );
        let (_r, _k, diags) = parse_style_block(".x { color: var(--a }");
        assert!(
            diags.iter().any(|d| d.message.contains("unbalanced")),
            "{diags:?}"
        );
    }

    #[test]
    fn parse_custom_prop_block_cycle_warns() {
        // 同块静态可见的环 → warning（非 error，规则照常产出——运行时该环 invalid）
        let (rules, _kf, diags) = parse_style_block("div { --a: var(--b) } div { --b: var(--a) }");
        assert_eq!(rules.len(), 2, "环是 warning 不拦规则");
        assert!(
            diags
                .iter()
                .any(|d| d.severity == crate::diagnostic::Severity::Warning
                    && d.code == DiagnosticCode::FenceCustomPropCycle),
            "{diags:?}"
        );
    }

    #[test]
    fn runtime_css_parse_ok_and_at_rules_rejected() {
        // 合面：普通规则 + custom prop + var 值全收。
        let rules = parse_runtime_css(
            ".rt { --accent: #ff0000 } .rt .target { color: var(--accent, #888) }",
        )
        .expect("合法注入应 Ok");
        assert_eq!(rules.len(), 2);
        // at-rule 全拒（含 @keyframes——合法打包期形态在注入通道也是错）。
        for bad in [
            "@keyframes fade { from{opacity:0} to{opacity:1} }",
            "@media screen { .x { color:#fff } }",
        ] {
            let err = parse_runtime_css(bad).expect_err("at-rule 应被注入通道拒绝");
            assert!(
                err.message.contains("runtime stylesheet"),
                "{}",
                err.message
            );
            assert!(err.location.line >= 1, "行列信息在场");
        }
    }

    #[test]
    fn runtime_css_parse_bad_selector_and_prop_error_with_location() {
        let err = parse_runtime_css(".a + .b { color: #fff }").expect_err("越界选择器");
        assert!(err.message.contains(".a + .b"));
        let err = parse_runtime_css(".a { colr: #fff }").expect_err("未知 prop");
        assert!(err.message.contains("colr"));
    }

    #[test]
    fn runtime_css_empty_rules_ok() {
        // 空串/纯注释 → Ok(空规则集)（Clear 场景之外的空 Add 不算错）。
        assert!(parse_runtime_css("").unwrap().is_empty());
        assert!(parse_runtime_css("/* note */").unwrap().is_empty());
    }
}
