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

/// cssparser 后端。QualifiedRuleParser 产 Rule；AtRuleParser 默认拒（v0 无 @ 规则）。
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
    // v0 无 @ 规则，全部走默认拒实现
}

/// DeclParser 同时 impl 三个 trait（RuleBodyItemParser 的 super-trait 要求）。
/// QualifiedRuleParser/AtRuleParser 用默认拒实现——v0 声明块内不嵌套规则。
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
        false // 声明块内不嵌套规则（v0 无 nesting）
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
        rules.push(rule);
    }
    Ok(StyleSheet { rules })
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
}
