//! 运行时伪类重匹配的动态规则表（spec §5.5）。T3 只定义类型 + Default；
//! match_element_with_state + rematch_pseudo_classes 在 T6 填实。

use crate::parse::css::Declaration;
use crate::parse::selector::ParsedSelector;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DynamicRuleTable {
    pub rules: Vec<DynamicRule>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DynamicRule {
    pub selector: ParsedSelector,
    pub declarations: Vec<Declaration>,
}
