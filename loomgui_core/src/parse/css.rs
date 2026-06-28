use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser,
};

// Declaration 数据类型常驻在 `style::dynamic`（序列化进 .pkg.bin，runtime 不依赖 parse）。
// 本 parse-gated 模块重导出保持 `loomgui_core::parse::css::Declaration` 路径兼容。
pub use crate::style::dynamic::Declaration;

#[derive(Debug, Clone)]
pub struct StyleSheet {
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub selector_text: String, // 原始选择器文本，交 selector.rs 解析
    pub declarations: Vec<Declaration>,
}

/// cssparser 后端。QualifiedRuleParser 产 Rule；AtRuleParser 默认拒（无 @ 规则）。
struct NestingParser;

impl<'i> QualifiedRuleParser<'i> for NestingParser {
    type Prelude = String; // 选择器文本
    type QualifiedRule = Rule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        // prelude 在 `{` 前结束（parse_until_before），把整段切下来当选择器文本
        let start = input.position();
        while input.next().is_ok() {}
        Ok(input.slice_from(start).trim().to_string())
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        let mut declarations = Vec::new();
        let mut decl_parser = DeclParser;
        let body = RuleBodyParser::new(input, &mut decl_parser);
        for d in body.flatten() {
            declarations.push(d);
        }
        Ok(Rule {
            selector_text: prelude,
            declarations,
        })
    }
}

impl<'i> AtRuleParser<'i> for NestingParser {
    type Prelude = ();
    type AtRule = Rule;
    type Error = ();
    // 无 @ 规则，全部走默认拒实现
}

/// DeclParser 同时 impl 三个 trait（RuleBodyItemParser 的 super-trait 要求）。
/// QualifiedRuleParser/AtRuleParser 用默认拒实现——声明块内不嵌套规则。
struct DeclParser;

impl<'i> QualifiedRuleParser<'i> for DeclParser {
    type Prelude = ();
    type QualifiedRule = Declaration;
    type Error = ();
}

impl<'i> AtRuleParser<'i> for DeclParser {
    type Prelude = ();
    type AtRule = Declaration;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, Declaration, ()> for DeclParser {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false // 声明块内不嵌套规则（无 nesting）
    }
}

impl<'i> DeclarationParser<'i> for DeclParser {
    type Declaration = Declaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Declaration, ParseError<'i, ()>> {
        let start = input.position();
        while input.next().is_ok() {}
        let value = input.slice_from(start).trim().to_string();
        Ok(Declaration {
            prop: name.to_string(),
            value,
        })
    }
}

pub fn parse_css(css: &str) -> Result<StyleSheet, String> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut rules = Vec::new();
    // 用顶层 StyleSheetParser 迭代规则（比循环 parse_one_rule 更贴合 cssparser 0.34 设计）
    let mut nesting = NestingParser;
    let sheet = StyleSheetParser::new(&mut parser, &mut nesting);
    for rule in sheet.flatten() {
        // CSS 分组选择器 `.op,.tr{...}` 展开成多条 Rule（每组逗号隔开的选择器各一条，
        // 共享 declarations）。cssparser 把整段 prelude 当一条 selector_text，parse_selector
        // 不认逗号会把 `.op,.tr` 当单 compound（class=["op,","tr"]）→ 永不匹配。
        // 在此层展开最稳：parse_selector / match_element 无需感知逗号。
        let declarations = rule.declarations;
        for sel in rule
            .selector_text
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            rules.push(Rule {
                selector_text: sel.to_string(),
                declarations: declarations.clone(),
            });
        }
    }
    Ok(StyleSheet { rules })
}

/// 解析元素 inline `style="..."` 属性值（无 selector 的 declaration list，如
/// `"background-color:#1a1d2e; flex-direction:row"`）→ Declaration 列表。
/// 复用 DeclParser + RuleBodyParser（与 parse_block 同源）——inline style 本质即
/// declaration list，RuleBodyParser 不强制外层 `{}`。
pub fn parse_inline_style(style: &str) -> Vec<Declaration> {
    let mut input = ParserInput::new(style);
    let mut parser = Parser::new(&mut input);
    let mut decl_parser = DeclParser;
    let body = RuleBodyParser::new(&mut parser, &mut decl_parser);
    let mut decls = Vec::new();
    for d in body.flatten() {
        decls.push(d);
    }
    decls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_rule() {
        let ss = parse_css(".panel { width: 100px; color: red; }").unwrap();
        assert_eq!(ss.rules.len(), 1);
        assert_eq!(ss.rules[0].selector_text, ".panel");
        assert_eq!(ss.rules[0].declarations.len(), 2);
        assert_eq!(ss.rules[0].declarations[0].prop, "width");
        assert_eq!(ss.rules[0].declarations[0].value, "100px");
    }

    #[test]
    fn strips_whitespace_and_semicolons() {
        let ss = parse_css("div {  padding : 4px ; }").unwrap();
        assert_eq!(ss.rules[0].declarations[0].prop, "padding");
        assert_eq!(ss.rules[0].declarations[0].value, "4px");
    }

    #[test]
    fn parse_inline_style_parses_declarations() {
        // 色块 style="background-color:#1a1d2e" 等 inline 属性解析。
        let decls = parse_inline_style("background-color:#1a1d2e; flex-direction:row");
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].prop, "background-color");
        assert_eq!(decls[0].value, "#1a1d2e");
        assert_eq!(decls[1].prop, "flex-direction");
        assert_eq!(decls[1].value, "row");
    }

    /// CSS 分组选择器 `.op,.tr{...}` 必须展开成两条独立 Rule（CSS 标准语义）。
    /// 不展开的话 parse_selector 不认逗号 → `.op,.tr` 当单 compound，class 切成
    /// ["op,","tr"] → 要求元素同时含两个 → 永不匹配 → 规则整条失效。
    #[test]
    fn comma_group_selector_expands_to_multiple_rules() {
        let ss = parse_css(".op,.tr{width:80px;background-color:#5fb2c4}").unwrap();
        assert_eq!(ss.rules.len(), 2, ".op,.tr 展开为 2 条 Rule");
        let sels: Vec<&str> = ss.rules.iter().map(|r| r.selector_text.as_str()).collect();
        assert!(sels.contains(&".op"), "含 .op");
        assert!(sels.contains(&".tr"), "含 .tr");
        // 两条共享同 declarations（分组选择器语义）
        for r in &ss.rules {
            assert_eq!(r.declarations.len(), 2, "每条 Rule 含完整 declarations");
        }
    }

    /// 分组选择器第二条单独命中元素（端到端验证展开后两组各自独立匹配）。
    #[test]
    fn comma_group_second_selector_matches_element() {
        use crate::parse::dom::parse_html;
        let html = r#"<div class="root"><div class="tr"></div></div>"#;
        let css = ".op,.tr{background-color:#5fb2c4}";
        let tree = parse_html(html).unwrap();
        let sheet = parse_css(css).unwrap();
        let styles = crate::style::cascade::resolve_styles(&tree, &sheet);
        let tr_id = tree.nodes[tree.roots[0].0].children[0];
        let bg = styles[tr_id.0].background_color.expect(".tr 应有 bg");
        assert_eq!(bg, [0x5f as f32 / 255.0, 0xb2 as f32 / 255.0, 0xc4 as f32 / 255.0, 1.0]);
    }
}
