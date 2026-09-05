//! preview 组件 `<style>` 作用域改写（#95 / #94 前半步）——CSS 语义单真相侧。
//!
//! core 的样式墙靠 `ScopedRule{scope_root: host}`：选择器原样、作用域 = 组件子树
//! **含模板根**、host 本体不吃组件规则（shadow :host 语义）。浏览器没有等价物，
//! 旧实现是 expand.js 里的正则前缀器——只会拼后代选择器，根类规则整条死（#95），
//! 且 @media 放行、@keyframes 同名碰撞优先级反转（#95 审计批）。本模块把改写
//! 收编进 Rust，经 `/yio-preview/comp-style/<name>.css` 路由供给，客户端只注入
//! 链接。改写口径与 core/打包器逐条对齐：
//!
//! - **双分支**：每条规则输出后代分支 `[data-yio-comp="n"] sel`（子树内命中，
//!   含 slot 投射内容——它们物理上已在标记子树内）+ 根分支（属性插进首复合段，
//!   `div…` → `div[data-yio-comp="n"]…`、`.tip` → `[data-yio-comp="n"].tip`），
//!   与 core「子树含根、选择器原样」全等。
//! - **host 链剥标签**：`tip-panel.is-press .slot` 剥链首标签后按普通规则改写
//!   （浏览器近似里 host 的类已镜像到模板根上）。
//! - **fail-closed 对齐**：越界选择器（`>`/`+`/`~`/`*`/未知伪类等，判定复用
//!   fence `parse_selector`——与打包同一份选择器真相）与非 @keyframes at-rule
//!   （@media 等）整段丢弃 + 注释——preview 只显示可构建的真相，放行就是
//!   「预览能看、`yio build` 报错」的假象。
//! - **@keyframes 原文透传**；同名 keyframes 宿主优先由客户端注入次序实现
//!   （组件样式插在页面样式之前）。
//! - **声明块原文透传**（不经内部结构往返，浏览器所见即作者所写）；`url(相对
//!   路径)` 按组件文件位置绝对化成 `/ws/...`。

/// 改写一段组件 CSS（`<style>` 文本 + `<link rel=stylesheet>` 内容的拼接）为
/// 浏览器可用的作用域版本。`name` = 组件标签名（围栏保证 `[a-z0-9-]`，可安全
/// 内插进属性选择器）；`comp_rel` = 组件文件的工作区相对路径（url() 解析基准）。
pub(crate) fn rewrite_component_css(name: &str, css: &str, comp_rel: &str) -> String {
    let base_dir = parent_dir(comp_rel);
    let mut out = String::new();
    for seg in scan_segments(css) {
        match seg {
            Segment::Block { prelude, block } => {
                let prelude = prelude.trim();
                if let Some(at) = at_keyword(prelude) {
                    if at == "@keyframes" || at == "@-webkit-keyframes" {
                        out.push_str(prelude);
                        out.push_str(" {");
                        out.push_str(&block);
                        out.push_str("}\n");
                    } else {
                        // fence 对非 @keyframes at-rule 是打包 error——preview 丢弃并
                        // 留注释，与构建口径一致。
                        out.push_str(&format!(
                            "/* preview: at-rule `{at}` dropped — the fence only supports \
                             @keyframes; `yio build` errors on it */\n"
                        ));
                    }
                    continue;
                }
                rewrite_rule(name, prelude, &block, &base_dir, &mut out);
            }
            Segment::Statement { prelude } => {
                out.push_str(&format!(
                    "/* preview: blockless at-rule `{}` dropped */\n",
                    prelude.trim()
                ));
            }
        }
    }
    out
}

/// 顶层扫描产物：`prelude { block }` 或 `prelude;`（无块语句，如 @charset）。
/// 声明块内容不含首尾花括号；prelude 已剥注释、保留字符串原样。
enum Segment {
    Block { prelude: String, block: String },
    Statement { prelude: String },
}

/// 顶层段扫描：字符串/注释/括号感知。`(` 内的 `{`/`}`/`;` 不终结段（:nth-child
/// 参数等）；注释内容对 prelude 不可见（剥除）、对块原样保留（@yio-hook 锚点）。
fn scan_segments(css: &str) -> Vec<Segment> {
    let chars: Vec<char> = css.chars().collect();
    let mut segs = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut prelude = String::new();
        let mut depth = 0i32;
        let mut terminated = None;
        while i < chars.len() {
            let c = chars[i];
            match c {
                '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                    i = skip_comment(&chars, i);
                }
                '"' | '\'' => {
                    let end = skip_string(&chars, i);
                    prelude.extend(&chars[i..end]);
                    i = end;
                }
                '(' => {
                    depth += 1;
                    prelude.push(c);
                    i += 1;
                }
                ')' => {
                    depth -= 1;
                    prelude.push(c);
                    i += 1;
                }
                '{' if depth <= 0 => {
                    terminated = Some('{');
                    i += 1;
                    break;
                }
                ';' if depth <= 0 => {
                    terminated = Some(';');
                    i += 1;
                    break;
                }
                _ => {
                    prelude.push(c);
                    i += 1;
                }
            }
        }
        match terminated {
            Some('{') => {
                let mut block = String::new();
                let mut bdepth = 1i32;
                while i < chars.len() {
                    let c = chars[i];
                    match c {
                        '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                            let end = skip_comment(&chars, i);
                            block.extend(&chars[i..end]);
                            i = end;
                        }
                        '"' | '\'' => {
                            let end = skip_string(&chars, i);
                            block.extend(&chars[i..end]);
                            i = end;
                        }
                        '{' => {
                            bdepth += 1;
                            block.push(c);
                            i += 1;
                        }
                        '}' => {
                            bdepth -= 1;
                            if bdepth == 0 {
                                i += 1;
                                break;
                            }
                            block.push(c);
                            i += 1;
                        }
                        _ => {
                            block.push(c);
                            i += 1;
                        }
                    }
                }
                segs.push(Segment::Block { prelude, block });
            }
            Some(';') | None | Some(_) => {
                // 无块语句（@charset 等）与 EOF 截断的尾巴：非空才标注丢弃，纯空白静默。
                if !prelude.trim().is_empty() {
                    segs.push(Segment::Statement { prelude });
                }
            }
        }
    }
    segs
}

/// 跳过 `/* ... */`（含未闭合吞到 EOF）。`i` 指向首个 `/`，返回注释后位置。
fn skip_comment(chars: &[char], i: usize) -> usize {
    let mut j = i + 2;
    while j + 1 < chars.len() && !(chars[j] == '*' && chars[j + 1] == '/') {
        j += 1;
    }
    (j + 2).min(chars.len())
}

/// 跳过字符串字面量（`\` 转义感知）。`i` 指向开引号，返回闭引号后位置。
fn skip_string(chars: &[char], i: usize) -> usize {
    let q = chars[i];
    let mut j = i + 1;
    while j < chars.len() {
        if chars[j] == '\\' {
            j += 2;
            continue;
        }
        if chars[j] == q {
            return j + 1;
        }
        j += 1;
    }
    chars.len()
}

/// prelude 的 at-关键字（`@keyframes` / `@media` / …，小写归一）。非 at-rule 返 None。
fn at_keyword(prelude: &str) -> Option<String> {
    let rest = prelude.trim_start();
    let rest = rest.strip_prefix('@')?;
    let word: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if word.is_empty() {
        None
    } else {
        Some(format!("@{}", word.to_ascii_lowercase()))
    }
}

/// 一条元素规则 → 双分支输出。分支为空（全部段被丢弃）时只留注释。
fn rewrite_rule(name: &str, prelude: &str, block: &str, base_dir: &str, out: &mut String) {
    let attr = format!("[data-yio-comp=\"{name}\"]");
    let mut branches: Vec<String> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for part in split_commas(prelude) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let stripped = strip_host_led(name, part);
        if stripped.is_empty() {
            // 纯宿主选择器（`tip-panel`）：host 本体不吃组件规则，浏览器近似里
            // 标记在模板根上 → 规则落根（无更深的「宿主」可命中）。
            branches.push(attr.clone());
            continue;
        }
        // 宿主直子链（`tip-panel > .slot`，剥宿主后以 `>` 开头——#114 放行 Child）：
        // 原串整体过 fence 校验（剥掉的宿主标签占首个 compound），分支 = `{attr} {rest}`
        // （= 模板根的直接子；无「首复合段」可插，两分支同形）。
        if stripped.starts_with('>') {
            if yio_fence::css_rules::parse_selector(part).is_none() {
                dropped.push(part.to_string());
                continue;
            }
            branches.push(format!("{attr} {stripped}"));
            continue;
        }
        // fence 校验 = 与打包同一份选择器真相；越界构造 preview 同步拒绝。
        if yio_fence::css_rules::parse_selector(stripped).is_none() {
            dropped.push(part.to_string());
            continue;
        }
        // 后代分支：子树内（含 slot 投射内容——物理上已在标记子树内）。
        branches.push(format!("{attr} {stripped}"));
        // 根分支：属性插进首复合段（fence 校验过的 compound，tag 若有必在最前）。
        let compounds = split_compounds(stripped);
        let mut root = insert_attr_in_compound(&attr, compounds[0]);
        for c in &compounds[1..] {
            root.push(' ');
            root.push_str(c);
        }
        branches.push(root);
    }
    for d in &dropped {
        out.push_str(&format!(
            "/* preview: selector `{d}` dropped — outside the fence; `yio build` errors on it */\n"
        ));
    }
    if branches.is_empty() {
        return;
    }
    out.push_str(&branches.join(", "));
    out.push_str(" {");
    out.push_str(&absolutize_urls(block, base_dir));
    out.push_str("}\n");
}

/// 剥离宿主标签开头的链首段：`tip-panel.is-press .slot` → `.is-press .slot`、
/// `tip-panel` → 空。要求标签后紧跟边界符（`.`/`#`/`[`/`:`/空白/结尾），防
/// `tip-panel-x` 误剥。core 侧这条匹配 host 节点再下探；预览近似 = 剥掉后由
/// 模板根（已镜像 host 类）承接。
fn strip_host_led<'a>(name: &str, part: &'a str) -> &'a str {
    if let Some(rest) = part.strip_prefix(name) {
        match rest.chars().next() {
            None | Some('.') | Some('#') | Some('[') | Some(':') => return rest.trim_start(),
            Some(c) if c.is_whitespace() => return rest.trim_start(),
            _ => {}
        }
    }
    part
}

/// 顶层逗号切分（括号/字符串/注释感知）。注释已被 prelude 扫描剥除，这里只需
/// 防字符串与括号内的逗号。
fn split_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '"' | '\'' => {
                i = skip_string(&chars, i);
            }
            '(' => {
                depth += 1;
                i += 1;
            }
            ')' => {
                depth -= 1;
                i += 1;
            }
            ',' if depth <= 0 => {
                // 字节切片安全：',' 是 ASCII，UTF-8 自同步。
                parts.push(&s[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    parts.push(&s[start..]);
    parts
}

/// 复合段切分：括号深度 0 处的空白分段（与 fence parse_selector 同一算法——
/// 入参已过 fence 校验，保证一致）。
fn split_compounds(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '(' => {
                depth += 1;
                i += 1;
            }
            ')' => {
                depth -= 1;
                i += 1;
            }
            c if c.is_whitespace() && depth <= 0 => {
                if i > start {
                    parts.push(&s[start..i]);
                }
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

/// 属性选择器插进复合段：tag 在前则插 tag 后（`div` → `div[attr]`——CSS 复合段
/// 语法要求 type selector 居首），否则前缀（`.tip` → `[attr].tip`）。`*` 不在
/// fence 子集，到不了这里。
fn insert_attr_in_compound(attr: &str, compound: &str) -> String {
    let mut tag_end = 0usize;
    for (i, c) in compound.char_indices() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            tag_end = i + c.len_utf8();
        } else {
            break;
        }
    }
    let mut out = String::with_capacity(compound.len() + attr.len());
    out.push_str(&compound[..tag_end]);
    out.push_str(attr);
    out.push_str(&compound[tag_end..]);
    out
}

/// 声明块里的 `url(相对路径)` 按组件文件目录绝对化成 `/ws/...`；绝对路径/
/// scheme/data:/锚点/查询一概不动（宁漏改不误伤）。其余字节原样。
fn absolutize_urls(block: &str, base_dir: &str) -> String {
    let lower = block.to_ascii_lowercase();
    let mut out = String::new();
    let mut i = 0usize;
    while let Some(off) = lower[i..].find("url(") {
        let at = i + off;
        out.push_str(&block[i..at + 4]);
        let open = at + 4;
        let Some(close_rel) = block[open..].find(')') else {
            out.push_str(&block[open..]);
            return out;
        };
        let close = open + close_rel;
        let token = block[open..close].trim();
        out.push_str(&rewrite_url_token(token, base_dir).unwrap_or_else(|| token.to_string()));
        out.push(')');
        i = close + 1;
    }
    out.push_str(&block[i..]);
    out
}

/// 单个 url() 内容改写：引号保持、朴素相对路径解析，不可解析/不该动返 None。
fn rewrite_url_token(token: &str, base_dir: &str) -> Option<String> {
    let (quote, inner) = match token.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        Some(s) => (Some('"'), s),
        None => match token.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
            Some(s) => (Some('\''), s),
            None => (None, token),
        },
    };
    if inner.is_empty()
        || inner.starts_with('/')
        || inner.starts_with('#')
        || inner.contains(':')
        || inner.contains('?')
        || inner.contains('#')
    {
        return None;
    }
    let resolved = resolve_ws_rel(base_dir, inner)?;
    let ws = format!("/ws/{resolved}");
    Some(match quote {
        Some(q) => format!("{q}{ws}{q}"),
        None => ws,
    })
}

/// 工作区相对路径拼接（`..` 回退、`.` 跳过）；逃出工作区根或含点开头段（/ws/
/// 沙箱会拒）返 None——调用方保留原样。
pub(crate) fn resolve_ws_rel(base_dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for seg in base_dir.split('/') {
        if !seg.is_empty() && seg != "." {
            parts.push(seg);
        }
    }
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s if s.starts_with('.') => return None,
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// `a/b/c.html` → `a/b`（无目录 → 空）。
fn parent_dir(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "tip-panel";
    const REL: &str = "showcase/components/tip-panel.html";
    const ATTR: &str = "[data-yio-comp=\"tip-panel\"]";

    fn rw(css: &str) -> String {
        rewrite_component_css(NAME, css, REL)
    }

    #[test]
    fn root_class_rule_gets_root_branch() {
        // #95 本体：根类规则必须同时输出根匹配分支（后代分支永远够不着标记根）。
        let out = rw(".tip { background: #eee; width: 320px; }");
        assert!(out.contains(&format!("{ATTR} .tip, {ATTR}.tip {{")));
        assert!(out.contains("width: 320px"));
    }

    #[test]
    fn tag_led_rule_inserts_attr_after_tag() {
        // CSS 复合段语法：type selector 必须居首 → 属性插 tag 后。
        let out = rw("div { color: red }");
        assert!(out.contains(&format!("{ATTR} div, div{ATTR} {{ color: red }}")));
    }

    #[test]
    fn comma_list_each_part_dual_branch() {
        let out = rw(".a, .b { x: 1 }");
        assert!(out.contains(&format!(
            "{ATTR} .a, {ATTR}.a, {ATTR} .b, {ATTR}.b {{ x: 1 }}"
        )));
    }

    #[test]
    fn host_led_chain_strips_tag() {
        let out = rw("tip-panel.is-press .slot { color: red }");
        assert!(out.contains(&format!("{ATTR} .is-press .slot, {ATTR}.is-press .slot {{")));
    }

    #[test]
    fn bare_host_selector_lands_on_root() {
        let out = rw("tip-panel { border: 1px }");
        assert!(out.contains(&format!("{ATTR} {{ border: 1px }}")));
        // 近名不剥：tip-panel-x 是另一个标签。
        let out2 = rw("tip-panel-x { border: 1px }");
        assert!(out2.contains(&format!("{ATTR} tip-panel-x, tip-panel-x{ATTR} {{")));
    }

    #[test]
    fn chain_with_root_first_compound() {
        // 链中首复合段落在根上：`.a .b` 的 `.a` 可能就是根自己。
        let out = rw(".a .b { x: 1 }");
        assert!(out.contains(&format!("{ATTR} .a .b, {ATTR}.a .b {{ x: 1 }}")));
    }

    #[test]
    fn pseudo_and_nth_child_keep_parens_intact() {
        let out = rw(".x:hover { x: 1 }");
        assert!(out.contains(&format!("{ATTR} .x:hover, {ATTR}.x:hover {{")));
        // 括号内空格不拆复合段（fence An+B 语法）。
        let out2 = rw(":nth-child(2n + 1) { x: 1 }");
        assert!(out2.contains(&format!(
            "{ATTR} :nth-child(2n + 1), {ATTR}:nth-child(2n + 1) {{"
        )));
    }

    #[test]
    fn out_of_fence_selector_dropped_with_comment() {
        // `.a + .b` 越界（`>` 已入子集，#114）→ 该规则单独丢弃并留注释。
        let out = rw(".a + .b { color: red }");
        assert!(!out.contains("color: red"));
        assert!(out.contains("selector `.a + .b` dropped"));
        // 混合逗号：合法段保留，越界段单独丢弃。
        let out2 = rw(".a, .b + .c { x: 1 }");
        assert!(out2.contains(&format!("{ATTR} .a, {ATTR}.a {{ x: 1 }}")));
        assert!(out2.contains("`.b + .c` dropped"));
    }

    #[test]
    fn child_combinator_passes_through_both_branches() {
        // #114：`>` 已入子集——双分支保留 Child 语义（scope 属性钉首复合段 = 直父）。
        let out = rw(".a > .b { color: red }");
        assert!(out.contains(&format!("{ATTR} .a > .b, {ATTR}.a > .b {{ color: red }}")));
        // 紧凑写法同透传。
        let out2 = rw(".a>.b { x: 1 }");
        assert!(out2.contains(&format!("{ATTR} .a>.b, {ATTR}.a>.b {{ x: 1 }}")));
    }

    #[test]
    fn host_led_child_chain_keeps_direct_child_of_root() {
        // 宿主直子链 `tip-panel > .slot`：剥宿主后以 `>` 开头——两分支同形，
        // 语义 = 模板根的直接子（不得把属性插进 `>` 造出 `{attr} > > .slot`）。
        let out = rw("tip-panel > .slot { x: 1 }");
        assert!(out.contains(&format!("{ATTR} > .slot {{ x: 1 }}")));
        assert!(!out.contains("> >"));
    }

    #[test]
    fn keyframes_pass_through_verbatim() {
        let css =
            "@keyframes spin { from { transform: rotate(0deg) } to { transform: rotate(360deg) } }";
        let out = rw(css);
        assert!(out.contains(css));
        // 前缀改写不碰 keyframes 帧（0%/from/to 不是元素选择器——旧 JS 正则的坑）。
        assert!(!out.contains(&format!("{ATTR} from")));
    }

    #[test]
    fn media_and_blockless_at_rules_dropped() {
        let out = rw("@media (max-width: 600px) { .a { color: red } }");
        assert!(!out.contains("color: red"));
        assert!(out.contains("at-rule `@media` dropped"));
        let out2 = rw("@charset \"utf-8\";\n.a { x: 1 }");
        assert!(out2.contains("blockless at-rule `@charset \"utf-8\"` dropped"));
        assert!(out2.contains(&format!("{ATTR} .a,")));
    }

    #[test]
    fn braces_inside_comments_and_strings_survive() {
        // 注释里的花括号不终结段；字符串里的花括号不当块边界。
        let out = rw("/* } { */\n[data-x=\"a{b\"] { x: 1 }");
        assert!(out.contains(&format!(
            "{ATTR} [data-x=\"a{{b\"], {ATTR}[data-x=\"a{{b\"] {{ x: 1 }}"
        )));
    }

    #[test]
    fn url_relativized_against_component_dir() {
        let out = rw(".bg { background-image: url(img/bg.png) }");
        assert!(out.contains("url(/ws/showcase/components/img/bg.png)"));
        // 引号保持；绝对/scheme/data 不动。
        let out2 = rw(".b { background-image: url(\"img/q.png\") }");
        assert!(out2.contains("url(\"/ws/showcase/components/img/q.png\")"));
        let out3 = rw(".c { background-image: url(https://x/y.png) }");
        assert!(out3.contains("url(https://x/y.png)"));
        let out4 = rw(".d { background-image: url(data:image/png;base64,AAAA) }");
        assert!(out4.contains("url(data:image/png;base64,AAAA)"));
    }

    #[test]
    fn resolve_ws_rel_basics() {
        assert_eq!(
            resolve_ws_rel("showcase/components", "img/bg.png").as_deref(),
            Some("showcase/components/img/bg.png")
        );
        assert_eq!(
            resolve_ws_rel("showcase/components", "../res/x.png").as_deref(),
            Some("showcase/res/x.png")
        );
        // 两个 `..` 只是回到工作区根，合法；三个才逃逸。
        assert_eq!(
            resolve_ws_rel("showcase/components", "../../x.png").as_deref(),
            Some("x.png")
        );
        assert_eq!(
            resolve_ws_rel("showcase/components", "../../../x.png"),
            None
        );
        assert_eq!(resolve_ws_rel("showcase/components", ".hidden.png"), None);
    }
}
