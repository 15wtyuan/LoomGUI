//! 包格式（.pkg.bin，当前 version=11）：Rust-internal（packager 写、runtime 读，C# 不解析）。
//!
//! v1.4-a 多组件格式（推翻 v1 单树）：一个 pkg.bin = 多个具名组件（ComponentTable 切分）。
//! 布局：Header(20B) + StringTable + ComponentTable + NodeBlock + PerComponentDynamicRules +
//!       AssetManifest。
//!   - Header 砍 root_w/root_h（root_size 归 Stage）+ 砍 atlas 引用（图集归 Unity）。
//!   - StringTable：组件名 / text content / img path / classes / id_attr 共用一张表（intern 去重）。
//!   - ComponentTable：每组件 {name_idx, root_node_idx, node_count, dynamic_rules_blob_len}。
//!   - NodeBlock：所有组件节点平铺，parent_idx 用 -1 表组件根（全局位置索引）。
//!   - PerComponentDynamicRules：每组件 dynamic_rules 的 bincode blob（紧跟 ComponentTable 段）。
//!   - AssetManifest：本包所有 img path（Unity 校验 res 齐全用）。
//! style 字段 = bincode(ResolvedStyle，已 bake)。img src 指向归一化 path 字符串（非 atlas sprite）。

use crate::scene::NodeKind;
use crate::style::dynamic::DynamicRuleTable;
use crate::style::resolved::ResolvedStyle;

pub mod texture; // 纹理注册表（src→TexMeta）—— v1.4-a 暂留（图集归 Unity，T4 删）

pub const PKG_MAGIC: u32 = 0x474B504C; // 磁盘字节(LE) "LPKG"（不与 frame blob "LOOM" 撞）
pub const PKG_FORMAT_VERSION: u32 = 11; // v1.4-a 多组件格式（ComponentTable + AssetManifest，砍 atlas/root_size，旧 v10 pkg 须重打）
pub(crate) const MIN_VERSION: u32 = 11;
pub(crate) const MAX_VERSION: u32 = 11;
const NULL_IDX: u16 = 0xFFFF;

const KIND_CONTAINER: u8 = 0;
const KIND_BUTTON: u8 = 1;
const KIND_IMAGE: u8 = 2;
const KIND_TEXT: u8 = 3;

/// atlas 元数据（.pkg.bin AtlasSection 的内存形）。
/// 单图集：atlas.len()≤1，所有 sprite 属 atlas[0]（build_registry 硬编码 tex_id=1）。
/// 多图集需 sprite 带 atlas_idx 才能分流——当前模型不支持。
#[derive(Debug, Clone, PartialEq)]
pub struct AtlasSection {
    pub atlases: Vec<AtlasInfo>,   // len=1
    pub sprites: Vec<AtlasSprite>, // 全部 sprite（都属 atlas 0）
}
impl Default for AtlasSection {
    fn default() -> Self {
        AtlasSection {
            atlases: Vec::new(),
            sprites: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AtlasInfo {
    pub filename: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AtlasSprite {
    pub src: String, // 原 Image src（NodeBlock image_src 一致）
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// 从 AtlasSection 建 TextureView registry。单图集：atlas[0] 得 tex_id=1，
/// 所有 sprite 属它。空 atlas（inline / 空 package）→ 空 registry。
/// （多图集需 sprite 带 atlas_idx 才能分流。）
pub fn build_registry(section: &AtlasSection) -> crate::asset::texture::TextureRegistry {
    let mut reg = crate::asset::texture::TextureRegistry::default();
    if section.atlases.is_empty() {
        return reg;
    }
    let atlas = &section.atlases[0];
    let atlas_tex_id = 1u32;
    let aw = atlas.width as f32;
    let ah = atlas.height as f32;
    for spr in &section.sprites {
        // UV 存 Unity 约定（v=0 底 / v=1 顶）：PNG y=0 顶 ↔ Unity v=1，故 v 翻转。
        // 旧用 PNG y-down（uv_min[1]=spr.y/ah），sprite 不占整高时 design 顶↔PNG 底（上下错位）；
        // v1.3 加 108px skin.png 打破 atlas 高度（icon 不再占满高）首次暴露。
        let uv_min = [spr.x as f32 / aw, (ah - (spr.y + spr.h) as f32) / ah];
        let uv_max = [(spr.x + spr.w) as f32 / aw, (ah - spr.y as f32) / ah];
        reg.insert(
            &spr.src,
            crate::asset::texture::TexMeta {
                tex_id: atlas_tex_id,
                uv_min,
                uv_max,
                width: spr.w,
                height: spr.h,
            },
        );
    }
    reg
}

// ── v1.4-a 多组件包数据结构 ──────────────────────────────────────────────

/// 一个已加载的包（资源池条目）。`name` read 时填空串，由 `Stage::load_package(name, ..)` 覆盖。
#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub components: std::collections::HashMap<String, ComponentTemplate>,
    pub asset_manifest: Vec<String>, // 本包用到的所有 img path（去重）
}

/// 一个组件的模板（instantiate 的克隆源）。
#[derive(Debug, Clone)]
pub struct ComponentTemplate {
    pub name: String,
    pub nodes: Vec<TemplateNode>,
    pub dynamic_rules: DynamicRuleTable,
}

/// 模板节点：序列化态（instantiate 时 build 成 live Node）。
/// 与 live Node 区别：无 NodeId（instantiate 时 slotmap 分配）、无 taffy_id（每帧 solve 重建）。
#[derive(Debug, Clone)]
pub struct TemplateNode {
    pub kind: NodeKind,
    pub style: ResolvedStyle, // base_style（已 bake）
    pub parent_idx: Option<usize>, // 模板内位置索引（None=组件根）
    pub classes: Vec<String>,
    pub id_attr: Option<String>,
    pub draggable: bool,
    pub tabindex: Option<i32>,
}

/// write_package 的输入（打包器构造，已归一化：path 已相对、style 已 bake）。
pub struct PackageInput<'a> {
    pub components: Vec<(&'a str, &'a [TemplateNode], &'a DynamicRuleTable)>,
    pub asset_manifest: &'a [String], // 已去重归一化的 path
}

#[derive(Debug)]
pub enum PkgError {
    BadMagic,
    TooOld(u32),
    TooNew(u32),
    Truncated(&'static str),
    OobString(u16),
    Bincode(bincode::Error),
    BadKind(u8),
    DupComponent(String),
}

impl std::fmt::Display for PkgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PkgError::BadMagic => write!(f, "bad magic (not a loom package)"),
            PkgError::TooOld(v) => {
                write!(f, "package formatVersion {v} too old (min {MIN_VERSION})")
            }
            PkgError::TooNew(v) => {
                write!(f, "package formatVersion {v} too new (max {MAX_VERSION})")
            }
            PkgError::Truncated(ctx) => write!(f, "truncated package: {ctx}"),
            PkgError::OobString(i) => write!(f, "string index {i} out of range"),
            PkgError::Bincode(e) => write!(f, "style bincode: {e}"),
            PkgError::BadKind(k) => write!(f, "bad node kind tag {k}"),
            PkgError::DupComponent(n) => {
                write!(f, "duplicate component name in package: {n}")
            }
        }
    }
}

impl std::error::Error for PkgError {}

impl From<bincode::Error> for PkgError {
    fn from(e: bincode::Error) -> Self {
        PkgError::Bincode(e)
    }
}

/// 从 StyleSheet 抽含 :hover/:active/:disabled/:focus 的规则 → DynamicRuleTable。
/// 纯静态规则不进（已在 base_style 烤好）。判定：parse_selector 后任一 compound 含伪类标志。
///
/// **parse-gated：**消费 parse 后的 StyleSheet（CSS 文本产物），runtime 无此输入。
#[cfg(feature = "parse")]
pub fn extract_dynamic_rules(sheet: &crate::parse::css::StyleSheet) -> DynamicRuleTable {
    use crate::parse::selector::parse_selector;
    use crate::style::dynamic::DynamicRule;
    let mut rules = Vec::new();
    for rule in &sheet.rules {
        let sel = match parse_selector(&rule.selector_text) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let has_pseudo = sel
            .compound
            .iter()
            .any(|c| c.pseudo_hover || c.pseudo_active || c.pseudo_disabled || c.pseudo_focus);
        if has_pseudo {
            rules.push(DynamicRule {
                selector: sel,
                declarations: rule.declarations.clone(),
            });
        }
    }
    DynamicRuleTable { rules }
}

// ── v1.4-a 归一化（T2：path + CSS）──────────────────────────────────────────
//
// 这两个是纯函数，T3 打包器扫 HTML 后调它们产归一化数据喂 write_package。
// 都 parse-gated：消费 parse 后的产物（scraper 解析 HTML / 读 css 文件），runtime 无此输入。

/// 归一化图片 src：去 res 目录前缀 + 统一正斜杠。
///
/// 输入例：`res/icons/skin.png` / `./res/icons/skin.png` / `res\icons\skin.png` → `icons/skin.png`。
/// `res_dir` 取自打包配置（默认 `res`，可配置，对应 spec D10）。
/// 不在 `res_dir/` 路径段下 → None（调用方 warning，不入 manifest）。
///
/// 边界检查：`res/` 必须是完整路径段，不误匹配 `pres/x`（前缀前字符须是 `/`、`.` 或串首）。
/// spec §3.4（D8/D10）。
#[cfg(feature = "parse")]
pub fn normalize_path(src: &str, res_dir: &str) -> Option<String> {
    let unified = src.replace('\\', "/"); // Win 反斜杠 → 正斜杠
    let prefix = format!("{}/", res_dir);
    // 找所有 res/ 出现位置，取第一个满足"前字符是 / 或 . 或串首"的（合法路径段）。
    let mut start = 0usize;
    while let Some(rel) = unified[start..].find(&prefix) {
        let abs = start + rel; // prefix 在 unified 中的绝对起点
        let before_ok = abs == 0 || matches!(unified.as_bytes()[abs - 1], b'/' | b'.');
        if before_ok {
            let stripped = &unified[abs + prefix.len()..];
            if !stripped.is_empty() {
                return Some(stripped.to_string());
            }
            // res/ 后空（如 "res/"）→ None
            return None;
        }
        start = abs + 1; // 跳过本次误匹配，继续找下一个
    }
    None
}

/// 抽 HTML 的所有 CSS 合并成 stylesheet 串：
/// - `<style>` 内联（取 text content，可多处）
/// - `<link rel="stylesheet" href="...">` 引用的 .css 文件内容（base_dir.join(href) 读）
///
/// 行内 `style=""` 不抽——由 `resolve_styles` 直接 bake 进节点 style（spec §3.5 D6）。
/// `base_dir` = HTML 文件所在目录，用于解析 `<link href>` 的相对路径。
///
/// **签名调整说明**：brief 伪写 `(tree: &DomTree, ...)`，但 `parse_html` 的围栏白名单
/// （`FENCE_TAGS = div/span/img/button`）会拒绝 `<style>`/`<link>`，无法从已解析 ElementTree
/// 取这俩节点。故直接吃原始 HTML 串、用 scraper 抽 `<style>`/`<link>`（与 dom.rs 同后端）。
/// T3 打包器在调 `parse_html` 之前先调本函数抽 CSS（同一份 HTML 串两用）。
#[cfg(feature = "parse")]
pub fn extract_component_css(html: &str, base_dir: &std::path::Path) -> String {
    use scraper::{Html, Selector};
    let document = Html::parse_document(html);
    let mut parts: Vec<String> = Vec::new();

    // <style> 内联：text content 整段收（scraper 的 text() 拼所有文本节点）。
    if let Ok(style_sel) = Selector::parse("style") {
        for el in document.select(&style_sel) {
            let text: String = el.text().collect();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }

    // <link rel="stylesheet" href="...">：读 href 文件内容。
    // rel 值大小写不敏感、允许空格分隔多值（如 "stylesheet preload"）——含 "stylesheet" 即抽。
    if let Ok(link_sel) = Selector::parse("link") {
        for el in document.select(&link_sel) {
            let rel_is_stylesheet = el
                .value()
                .attr("rel")
                .map(|r| r.split_whitespace().any(|t| t.eq_ignore_ascii_case("stylesheet")))
                .unwrap_or(false);
            if !rel_is_stylesheet {
                continue;
            }
            let Some(href) = el.value().attr("href") else { continue };
            let path = base_dir.join(href);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
            // 读失败（文件缺失/编码错）→ 静默跳过（打包期 warning 由调用方补，T3 事）
        }
    }

    parts.join("\n")
}

/// 序列化 PackageInput → .pkg.bin bytes（v1.4-a 多组件格式）。
///
/// 布局：Header(20B) + StringTable + ComponentTable + NodeBlock + PerComponentDynamicRules
///       + AssetManifest。所有字符串（组件名 / text / img path / classes / id_attr）
///       共用同一 StringTable（intern 去重）。`input` 须已归一化（path 相对、style bake）。
pub fn write_package(input: &PackageInput) -> Vec<u8> {
    // 1. intern 全部字符串（组件名 + 每节点 text/src/classes/id_attr + asset_manifest path）。
    //    所有 intern 必须在写 header(string_count) 之前完成。
    let mut strings: Vec<String> = Vec::new();
    let mut idx_of: std::collections::HashMap<String, u16> = std::collections::HashMap::new();

    let component_count = input.components.len();
    // 每组件：(name_idx, root_node_idx, node_count, dynamic_blob)
    // 全局 NodeBlock 由各组件节点顺次拼接，root_node_idx = 该组件首节点在全局的位置。
    let mut comp_records: Vec<(u16, u32, u32, Vec<u8>)> = Vec::with_capacity(component_count);
    // 每节点（全局）：(parent_idx:i32, kind_tag, style_blob, text_idx, src_idx, class_idx[], id_idx, flags, tabindex)
    let mut node_records: Vec<(i32, u8, Vec<u8>, u16, u16, Vec<u16>, u16, u8, i32)> = Vec::new();
    let mut global_node_offset: u32 = 0;
    for (name, nodes, dynamic_rules) in &input.components {
        let name_idx = intern(name, &mut strings, &mut idx_of);
        let comp_base = global_node_offset;
        // spec 约定 nodes[0]=组件根（parent=None)。debug_assert：write 输入由 T2/T3 控制，
        // 违反即打包器 bug（非运行时 malformed 输入），故 debug_assert 足够（release 不付代价）。
        if !nodes.is_empty() {
            debug_assert!(
                nodes[0].parent_idx.is_none(),
                "component `{name}` nodes[0] must be root (parent_idx=None)"
            );
        }
        // intern 每节点字符串 + 收 (parent_idx 全局化, ...)。spec 约定 nodes[0]=组件根（parent=None）。
        for tn in nodes.iter() {
            // parent_idx 是组件内局部位置；转全局（-1 = 组件根）
            let parent_global: i32 = match tn.parent_idx {
                None => -1,
                Some(p) => (comp_base as usize + p) as i32,
            };
            let (kind_tag, text_idx, src_idx) = match &tn.kind {
                NodeKind::Container => (KIND_CONTAINER, NULL_IDX, NULL_IDX),
                NodeKind::Button => (KIND_BUTTON, NULL_IDX, NULL_IDX),
                NodeKind::Image { src } => (KIND_IMAGE, NULL_IDX, intern(src, &mut strings, &mut idx_of)),
                NodeKind::Text { content } => (KIND_TEXT, intern(content, &mut strings, &mut idx_of), NULL_IDX),
            };
            let style_blob = bincode::serialize(&tn.style).expect("ResolvedStyle serializable");
            let class_idx: Vec<u16> = tn
                .classes
                .iter()
                .map(|c| intern(c, &mut strings, &mut idx_of))
                .collect();
            let id_idx = tn
                .id_attr
                .as_ref()
                .map(|id| intern(id, &mut strings, &mut idx_of))
                .unwrap_or(NULL_IDX);
            let flags: u8 = if tn.draggable { 0x01 } else { 0x00 };
            let tabindex = tn.tabindex.unwrap_or(i32::MIN);
            node_records.push((parent_global, kind_tag, style_blob, text_idx, src_idx, class_idx, id_idx, flags, tabindex));
        }
        let node_count = nodes.len() as u32;
        let dynamic_blob = bincode::serialize(dynamic_rules).expect("DynamicRuleTable serializable");
        comp_records.push((name_idx, comp_base, node_count, dynamic_blob));
        global_node_offset += node_count;
    }
    // intern asset_manifest path（供 AssetManifest 段引用 idx）
    let manifest_idx: Vec<u16> = input
        .asset_manifest
        .iter()
        .map(|p| intern(p, &mut strings, &mut idx_of))
        .collect();

    let mut out: Vec<u8> = Vec::new();
    // Header (20B): magic + version + flags + component_count + string_count
    out.extend_from_slice(&PKG_MAGIC.to_le_bytes());
    out.extend_from_slice(&PKG_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags
    out.extend_from_slice(&(component_count as u32).to_le_bytes());
    out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    // StringTable
    for s in &strings {
        let bytes = s.as_bytes();
        out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    // ComponentTable: 每组件 {name_idx(u16), root_node_idx(u32), node_count(u32), dynamic_rules_blob_len(u32)}
    for (name_idx, root_node_idx, node_count, dynamic_blob) in &comp_records {
        out.extend_from_slice(&name_idx.to_le_bytes());
        out.extend_from_slice(&root_node_idx.to_le_bytes());
        out.extend_from_slice(&node_count.to_le_bytes());
        out.extend_from_slice(&(dynamic_blob.len() as u32).to_le_bytes());
    }
    // NodeBlock: 每节点 {parent_idx(i32), kind_tag(u8), style_len(u32)+style_blob, text_idx(u16), src_idx(u16),
    //   class_count(u16)+class_idx[], id_idx(u16), flags(u8), tabindex(i32)}
    for (parent_idx, kind_tag, style_blob, text_idx, src_idx, class_idx, id_idx, flags, tabindex) in &node_records {
        out.extend_from_slice(&parent_idx.to_le_bytes());
        out.push(*kind_tag);
        out.extend_from_slice(&(style_blob.len() as u32).to_le_bytes());
        out.extend_from_slice(style_blob);
        out.extend_from_slice(&text_idx.to_le_bytes());
        out.extend_from_slice(&src_idx.to_le_bytes());
        out.extend_from_slice(&(class_idx.len() as u16).to_le_bytes());
        for &cidx in class_idx {
            out.extend_from_slice(&cidx.to_le_bytes());
        }
        out.extend_from_slice(&id_idx.to_le_bytes());
        out.push(*flags);
        out.extend_from_slice(&tabindex.to_le_bytes());
    }
    // PerComponentDynamicRules: 每组件 dynamic_blob（紧跟 ComponentTable 序）
    for (_, _, _, dynamic_blob) in &comp_records {
        out.extend_from_slice(&dynamic_blob);
    }
    // AssetManifest: path_count(u32) + path_idx[](u16)
    out.extend_from_slice(&(manifest_idx.len() as u32).to_le_bytes());
    for &pidx in &manifest_idx {
        out.extend_from_slice(&pidx.to_le_bytes());
    }
    out
}

/// 反序列化 .pkg.bin → Package（v1.4-a 多组件格式，含版本协商）。
/// `Package.name` read 时填空串（read 不知包名），由 `Stage::load_package(name, ..)` 覆盖。
pub fn read_package(bytes: &[u8]) -> Result<Package, PkgError> {
    let mut r = Reader::new(bytes);
    // Header (20B)
    let magic = r.u32("magic")?;
    if magic != PKG_MAGIC {
        return Err(PkgError::BadMagic);
    }
    let version = r.u32("version")?;
    if version < MIN_VERSION {
        return Err(PkgError::TooOld(version));
    }
    if version > MAX_VERSION {
        return Err(PkgError::TooNew(version));
    }
    let _flags = r.u32("flags")?;
    let component_count = r.u32("component_count")? as usize;
    let string_count = r.u32("string_count")? as usize;
    // StringTable
    let mut strings: Vec<String> = Vec::with_capacity(string_count);
    for _ in 0..string_count {
        let len = r.u16("str_len")? as usize;
        let s = r.utf8(len, "str_bytes")?;
        strings.push(s);
    }
    // ComponentTable: 每组件 {name_idx(u16), root_node_idx(u32), node_count(u32), dynamic_rules_blob_len(u32)}
    let mut comp_table: Vec<(u16, u32, u32, u32)> = Vec::with_capacity(component_count);
    for _ in 0..component_count {
        let name_idx = r.u16("comp_name_idx")?;
        let root_node_idx = r.u32("comp_root_node_idx")?;
        let node_count = r.u32("comp_node_count")?;
        let dynamic_len = r.u32("comp_dynamic_len")?;
        comp_table.push((name_idx, root_node_idx, node_count, dynamic_len));
    }
    // 总节点数 = 各组件 node_count 之和
    let total_nodes: u32 = comp_table.iter().map(|(_, _, n, _)| *n).sum();
    // NodeBlock → TemplateNode（平铺，parent_idx 存盘是全局位置；读后转回组件内局部）
    let mut all_nodes: Vec<TemplateNode> = Vec::with_capacity(total_nodes as usize);
    for _ in 0..total_nodes {
        let pidx = r.i32("parent_idx")?;
        let kind_tag = r.u8("kind")?;
        let style_len = r.u32("style_len")? as usize;
        let style: ResolvedStyle = bincode::deserialize(r.take(style_len, "style_blob")?)?;
        let text_idx = r.u16("text_idx")?;
        let src_idx = r.u16("src_idx")?;
        let class_count = r.u16("class_count")? as usize;
        let mut classes: Vec<String> = Vec::with_capacity(class_count);
        for _ in 0..class_count {
            let cidx = r.u16("class_idx")?;
            classes.push(string_at(&strings, cidx)?);
        }
        let id_idx = r.u16("id_idx")?;
        let id_attr = if id_idx == NULL_IDX {
            None
        } else {
            Some(string_at(&strings, id_idx)?)
        };
        let flags = r.u8("flags")?;
        let draggable = (flags & 0x01) != 0;
        let tab_raw = r.i32("tabindex")?;
        let tabindex = if tab_raw == i32::MIN { None } else { Some(tab_raw) };
        // 存盘 parent_idx 是 NodeBlock 全局位置（-1=组件根）；先存全局，待切分组件时减 base 转局部
        let parent_global = if pidx < 0 { None } else { Some(pidx as usize) };
        let kind = match kind_tag {
            KIND_CONTAINER => NodeKind::Container,
            KIND_BUTTON => NodeKind::Button,
            KIND_IMAGE => NodeKind::Image { src: string_at(&strings, src_idx)? },
            KIND_TEXT => NodeKind::Text { content: string_at(&strings, text_idx)? },
            other => return Err(PkgError::BadKind(other)),
        };
        all_nodes.push(TemplateNode {
            kind,
            style,
            parent_idx: parent_global, // 临时存全局，下方切分时减 base
            classes,
            id_attr,
            draggable,
            tabindex,
        });
    }
    // PerComponentDynamicRules: 每组件 dynamic_blob（按 ComponentTable 序）
    let mut components: std::collections::HashMap<String, ComponentTemplate> =
        std::collections::HashMap::with_capacity(component_count);
    for (name_idx, root_node_idx, node_count, dynamic_len) in &comp_table {
        let name = string_at(&strings, *name_idx)?;
        let start = *root_node_idx as usize;
        let end = start + *node_count as usize;
        // 防御 malformed ComponentTable：root_node_idx/node_count 越界 → Truncated（避免 slice panic）
        if start > all_nodes.len() || end > all_nodes.len() {
            return Err(PkgError::Truncated("comp_node_slice"));
        }
        let base = start;
        // 组件内 parent_idx：全局 - base（组件根 parent_idx=None 仍是 None）。
        // 防御 malformed：parent_global < base 表示父节点落到更早的组件 → Truncated（不允许跨组件父）
        let mut nodes = all_nodes[start..end].to_vec();
        for tn in nodes.iter_mut() {
            if let Some(p) = tn.parent_idx {
                if p < base {
                    return Err(PkgError::Truncated("cross_comp_parent"));
                }
                tn.parent_idx = Some(p - base);
            }
        }
        let dynamic_rules: DynamicRuleTable =
            bincode::deserialize(r.take(*dynamic_len as usize, "comp_dynamic_blob")?)?;
        // 防御 malformed：同名组件 → DupComponent（避免静默覆盖丢数据）
        if components.contains_key(&name) {
            return Err(PkgError::DupComponent(name));
        }
        components.insert(name.clone(), ComponentTemplate { name, nodes, dynamic_rules });
    }
    // AssetManifest: path_count(u32) + path_idx[](u16)
    let path_count = r.u32("manifest_path_count")? as usize;
    let mut asset_manifest: Vec<String> = Vec::with_capacity(path_count);
    for _ in 0..path_count {
        let pidx = r.u16("manifest_path_idx")?;
        asset_manifest.push(string_at(&strings, pidx)?);
    }
    Ok(Package { name: String::new(), components, asset_manifest })
}

fn string_at(strings: &[String], idx: u16) -> Result<String, PkgError> {
    if idx == NULL_IDX {
        return Ok(String::new());
    }
    strings
        .get(idx as usize)
        .cloned()
        .ok_or(PkgError::OobString(idx))
}

/// 把字符串 intern 进 stringTable（首次出现分配新索引，重复返回既有索引）。
fn intern(
    s: &str,
    strings: &mut Vec<String>,
    idx_of: &mut std::collections::HashMap<String, u16>,
) -> u16 {
    if let Some(&i) = idx_of.get(s) {
        return i;
    }
    let i = strings.len() as u16;
    strings.push(s.to_string());
    idx_of.insert(s.to_string(), i);
    i
}

/// 极简游标 reader：定长小端读取 + 截断保护。
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }
    fn need(&mut self, n: usize, ctx: &'static str) -> Result<&'a [u8], PkgError> {
        if self.pos + n > self.buf.len() {
            return Err(PkgError::Truncated(ctx));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self, ctx: &'static str) -> Result<u8, PkgError> {
        Ok(self.need(1, ctx)?[0])
    }
    fn u16(&mut self, ctx: &'static str) -> Result<u16, PkgError> {
        Ok(u16::from_le_bytes(
            self.need(2, ctx)?.try_into().unwrap(),
        ))
    }
    fn u32(&mut self, ctx: &'static str) -> Result<u32, PkgError> {
        Ok(u32::from_le_bytes(
            self.need(4, ctx)?.try_into().unwrap(),
        ))
    }
    fn i32(&mut self, ctx: &'static str) -> Result<i32, PkgError> {
        Ok(i32::from_le_bytes(
            self.need(4, ctx)?.try_into().unwrap(),
        ))
    }
    fn take(&mut self, n: usize, ctx: &'static str) -> Result<&'a [u8], PkgError> {
        self.need(n, ctx)
    }
    fn utf8(&mut self, n: usize, ctx: &'static str) -> Result<String, PkgError> {
        let s = self.need(n, ctx)?;
        std::str::from_utf8(s)
            .map(String::from)
            .map_err(|_| PkgError::Truncated(ctx))
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助：构造一个最小 TemplateNode（默认值）。
    fn tn(kind: NodeKind) -> TemplateNode {
        TemplateNode {
            kind,
            style: ResolvedStyle::default(),
            parent_idx: None,
            classes: vec![],
            id_attr: None,
            draggable: false,
            tabindex: None,
        }
    }

    /// 辅助：空 DynamicRuleTable 的稳定引用（避免临时生命周期）。
    fn empty_rules() -> DynamicRuleTable {
        DynamicRuleTable { rules: vec![] }
    }

    #[test]
    fn write_read_multi_component_roundtrip() {
        // 两组件：comp1 = root(parent=None) + child；comp2 单节点
        let mut tn_root = tn(NodeKind::Container);
        tn_root.id_attr = Some("r".into());
        let mut tn_child = tn(NodeKind::Text { content: "hi".into() });
        tn_child.parent_idx = Some(0);
        let comp1_nodes = vec![tn_root, tn_child];
        let comp2_nodes = vec![tn(NodeKind::Container)];
        let rules = empty_rules();
        let manifest = ["icons/skin.png".to_string()];
        let input = PackageInput {
            components: vec![
                ("comp1", comp1_nodes.as_slice(), &rules),
                ("comp2", comp2_nodes.as_slice(), &rules),
            ],
            asset_manifest: &manifest,
        };
        let bytes = write_package(&input);
        let pkg = read_package(&bytes).expect("read ok");
        assert_eq!(pkg.components.len(), 2);
        assert_eq!(pkg.components["comp1"].nodes.len(), 2);
        assert!(
            pkg.components["comp1"].nodes[1].parent_idx == Some(0),
            "child parent=root"
        );
        assert_eq!(pkg.asset_manifest, vec!["icons/skin.png".to_string()]);
    }

    #[test]
    fn old_version_pkg_rejected() {
        // 手构一个旧 version 的 header -> read_package 报 TooOld
        let mut old = vec![];
        old.extend_from_slice(&PKG_MAGIC.to_le_bytes());
        old.extend_from_slice(&(MIN_VERSION - 1).to_le_bytes()); // 旧 version
        assert!(matches!(read_package(&old), Err(PkgError::TooOld(_))));
    }

    #[test]
    fn read_rejects_bad_magic() {
        let mut bad = vec![0u8; 20];
        bad[0..4].copy_from_slice(&0x4D4F4F4Cu32.to_le_bytes());
        assert!(matches!(read_package(&bad), Err(PkgError::BadMagic)));
    }

    #[test]
    fn read_rejects_too_new_version() {
        let nodes = [tn(NodeKind::Container)];
        let rules = empty_rules();
        let input = PackageInput {
            components: vec![("c", &nodes, &rules)],
            asset_manifest: &[],
        };
        let mut bytes = write_package(&input);
        bytes[4..8].copy_from_slice(&(MAX_VERSION + 1).to_le_bytes());
        assert!(matches!(read_package(&bytes), Err(PkgError::TooNew(_))));
    }

    #[test]
    fn header_is_20_bytes_no_root_size() {
        // 新格式 header 20B（magic+version+flags+component_count+string_count），砍 root_w/root_h。
        let nodes = [tn(NodeKind::Container)];
        let rules = empty_rules();
        let input = PackageInput {
            components: vec![("c", &nodes, &rules)],
            asset_manifest: &[],
        };
        let bytes = write_package(&input);
        let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        assert_eq!(magic, PKG_MAGIC);
        let ver = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(ver, PKG_FORMAT_VERSION);
        let comp_count = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        assert_eq!(comp_count, 1);
        let sc = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        assert_eq!(sc, 1, "string_count 应在 offset 16（20B header）");
    }

    #[test]
    fn multi_component_parent_idx_is_component_local() {
        // 两个组件各 root + child：child parent_idx 应解析为各自组件内 0（局部），
        // 验证 write 全局化 + read 减 base 转局部。
        let mut root_a = tn(NodeKind::Container);
        root_a.id_attr = Some("a".into());
        let mut child_a = tn(NodeKind::Text { content: "ca".into() });
        child_a.parent_idx = Some(0);
        let mut root_b = tn(NodeKind::Container);
        root_b.id_attr = Some("b".into());
        let mut child_b = tn(NodeKind::Text { content: "cb".into() });
        child_b.parent_idx = Some(0);
        let comp_a = [root_a, child_a];
        let comp_b = [root_b, child_b];
        let rules = empty_rules();
        let input = PackageInput {
            components: vec![
                ("a", &comp_a, &rules),
                ("b", &comp_b, &rules),
            ],
            asset_manifest: &[],
        };
        let pkg = read_package(&write_package(&input)).unwrap();
        assert_eq!(pkg.components["a"].nodes[1].parent_idx, Some(0));
        assert_eq!(pkg.components["b"].nodes[1].parent_idx, Some(0));
        assert_eq!(pkg.components["a"].nodes[0].parent_idx, None);
        assert_eq!(pkg.components["b"].nodes[0].parent_idx, None);
    }

    #[test]
    fn all_node_kinds_roundtrip() {
        let mut img = tn(NodeKind::Image { src: "icons/a.png".into() });
        img.parent_idx = Some(0);
        let mut txt = tn(NodeKind::Text { content: "hello".into() });
        txt.parent_idx = Some(0);
        let nodes = [tn(NodeKind::Container), tn(NodeKind::Button), img, txt];
        let rules = empty_rules();
        let manifest = ["icons/a.png".to_string()];
        let input = PackageInput {
            components: vec![("c", &nodes, &rules)],
            asset_manifest: &manifest,
        };
        let pkg = read_package(&write_package(&input)).unwrap();
        let ns = &pkg.components["c"].nodes;
        assert!(matches!(ns[0].kind, NodeKind::Container));
        assert!(matches!(ns[1].kind, NodeKind::Button));
        assert!(matches!(&ns[2].kind, NodeKind::Image { src } if src == "icons/a.png"));
        assert!(matches!(&ns[3].kind, NodeKind::Text { content } if content == "hello"));
        assert_eq!(pkg.asset_manifest, vec!["icons/a.png".to_string()]);
    }

    #[test]
    fn classes_id_attr_draggable_tabindex_roundtrip() {
        let mut root = tn(NodeKind::Container);
        root.classes = vec!["a".into(), "b".into()];
        root.id_attr = Some("x".into());
        let mut btn = tn(NodeKind::Button);
        btn.parent_idx = Some(0);
        btn.draggable = true;
        btn.tabindex = Some(3);
        let nodes = [root, btn];
        let rules = empty_rules();
        let input = PackageInput {
            components: vec![("c", &nodes, &rules)],
            asset_manifest: &[],
        };
        let pkg = read_package(&write_package(&input)).unwrap();
        let ns = &pkg.components["c"].nodes;
        assert_eq!(ns[0].classes, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(ns[0].id_attr.as_deref(), Some("x"));
        assert!(!ns[0].draggable);
        assert_eq!(ns[0].tabindex, None);
        assert!(ns[1].draggable, "btn draggable=true round-trip");
        assert_eq!(ns[1].tabindex, Some(3));
    }

    #[test]
    fn style_blob_roundtrips_baked_resolved_style() {
        let mut n = tn(NodeKind::Container);
        n.style.background_color = Some([1.0, 0.0, 0.0, 1.0]);
        let nodes = [n];
        let rules = empty_rules();
        let input = PackageInput {
            components: vec![("c", &nodes, &rules)],
            asset_manifest: &[],
        };
        let pkg = read_package(&write_package(&input)).unwrap();
        let n2 = &pkg.components["c"].nodes[0];
        assert_eq!(n2.style.background_color, Some([1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn per_component_dynamic_rules_roundtrip() {
        use crate::parse::css::Declaration;
        use crate::parse::selector::parse_selector;
        use crate::style::dynamic::{DynamicRule, DynamicRuleTable};
        let rules_a = DynamicRuleTable {
            rules: vec![DynamicRule {
                selector: parse_selector(".a:hover").unwrap(),
                declarations: vec![Declaration { prop: "background-color".into(), value: "#f00".into() }],
            }],
        };
        let rules_b = DynamicRuleTable {
            rules: vec![DynamicRule {
                selector: parse_selector(".b:active").unwrap(),
                declarations: vec![Declaration { prop: "color".into(), value: "#00f".into() }],
            }],
        };
        let na = [tn(NodeKind::Container)];
        let nb = [tn(NodeKind::Container)];
        let input = PackageInput {
            components: vec![
                ("a", &na, &rules_a),
                ("b", &nb, &rules_b),
            ],
            asset_manifest: &[],
        };
        let pkg = read_package(&write_package(&input)).unwrap();
        assert_eq!(pkg.components["a"].dynamic_rules.rules.len(), 1);
        assert!(pkg.components["a"].dynamic_rules.rules[0].selector.compound[0].pseudo_hover);
        assert_eq!(pkg.components["b"].dynamic_rules.rules.len(), 1);
        assert!(pkg.components["b"].dynamic_rules.rules[0].selector.compound[0].pseudo_active);
    }

    #[test]
    fn stringtable_dedups_across_components() {
        // 两组件共用同 content "dup" -> StringTable 去重（string_count=3: "dup","c1","c2"）
        let mut n1 = tn(NodeKind::Text { content: "dup".into() });
        n1.id_attr = Some("c1".into());
        let mut n2 = tn(NodeKind::Text { content: "dup".into() });
        n2.id_attr = Some("c2".into());
        let c1_nodes = [n1];
        let c2_nodes = [n2];
        let rules = empty_rules();
        let input = PackageInput {
            components: vec![
                ("c1", &c1_nodes, &rules),
                ("c2", &c2_nodes, &rules),
            ],
            asset_manifest: &[],
        };
        let bytes = write_package(&input);
        let sc = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        assert_eq!(sc, 3, "重复 content 应跨组件去重");
        let pkg = read_package(&bytes).unwrap();
        assert!(matches!(&pkg.components["c1"].nodes[0].kind, NodeKind::Text { content } if content == "dup"));
        assert!(matches!(&pkg.components["c2"].nodes[0].kind, NodeKind::Text { content } if content == "dup"));
    }

    #[test]
    fn empty_package_roundtrips() {
        let rules = empty_rules();
        let input = PackageInput { components: vec![], asset_manifest: &[] };
        let _ = &rules; // 占位保持 lifetime 分析简单
        let pkg = read_package(&write_package(&input)).unwrap();
        assert_eq!(pkg.components.len(), 0);
        assert!(pkg.asset_manifest.is_empty());
        assert_eq!(pkg.name, "");
    }

    #[test]
    fn asset_manifest_multiple_paths_roundtrip() {
        let nodes = [
            tn(NodeKind::Image { src: "a/x.png".into() }),
            tn(NodeKind::Image { src: "b/y.png".into() }),
        ];
        let rules = empty_rules();
        let manifest = ["a/x.png".to_string(), "b/y.png".to_string()];
        let input = PackageInput {
            components: vec![("c", &nodes, &rules)],
            asset_manifest: &manifest,
        };
        let pkg = read_package(&write_package(&input)).unwrap();
        assert_eq!(pkg.asset_manifest, vec!["a/x.png".to_string(), "b/y.png".to_string()]);
    }

    // —— build_registry / AtlasSection 旧结构测试保留（v1.4-a 暂留这些类型，T4 删）——

    #[test]
    fn build_registry_maps_sprites_to_atlas_zero_tex_id_one() {
        let section = AtlasSection {
            atlases: vec![AtlasInfo { filename: "a.png".into(), width: 512, height: 256 }],
            sprites: vec![
                AtlasSprite { src: "s1.png".into(), x: 0, y: 0, w: 64, h: 32 },
                AtlasSprite { src: "s2.png".into(), x: 64, y: 32, w: 100, h: 200 },
            ],
        };
        let reg = build_registry(&section);
        let m1 = reg.get("s1.png").expect("s1 registered");
        assert_eq!(m1.tex_id, 1);
        assert_eq!(m1.uv_min, [0.0, (256.0 - 32.0) / 256.0]);
        assert_eq!(m1.uv_max, [64.0 / 512.0, (256.0 - 0.0) / 256.0]);
        assert_eq!((m1.width, m1.height), (64, 32));
    }

    #[test]
    fn build_registry_empty_atlases_returns_empty() {
        let section = AtlasSection::default();
        let reg = build_registry(&section);
        assert!(reg.is_empty());
    }

    // —— 防御 malformed ComponentTable 测试（review fix）——

    /// 辅助：计算 ComponentTable 段在 pkg bytes 中的起始 offset。
    /// 布局：Header(20B) + StringTable(每串 u16 len + bytes)。返回 ComponentTable 首字节 offset。
    fn comp_table_offset(bytes: &[u8]) -> usize {
        assert!(bytes.len() >= 20);
        let string_count = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let mut off = 20usize;
        for _ in 0..string_count {
            let len = u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap()) as usize;
            off += 2 + len;
        }
        off
    }

    /// 辅助：构造一个 2 组件 pkg（comp_a: root+child，comp_b: root），返回 bytes。
    /// 用于 patch 出 malformed 输入测 read_package 防御。
    fn two_comp_pkg_bytes() -> Vec<u8> {
        let mut root_a = tn(NodeKind::Container);
        root_a.id_attr = Some("a".into());
        let mut child_a = tn(NodeKind::Text { content: "ca".into() });
        child_a.parent_idx = Some(0);
        let comp_a = [root_a, child_a];
        let comp_b = [tn(NodeKind::Container)];
        let rules = empty_rules();
        let input = PackageInput {
            components: vec![("a", &comp_a, &rules), ("b", &comp_b, &rules)],
            asset_manifest: &[],
        };
        write_package(&input)
    }

    /// Important 1：malformed ComponentTable 的 root_node_idx/node_count 越界 → Truncated（不 panic）。
    /// 构造 comp_a 声称 root_node_idx=2, node_count=2，全 NodeBlock 有 3 节点 → slice [2..4] end=4 > 3 越界。
    /// （node_count 不改成 10：那样 total_nodes=11 跑 NodeBlock 循环时先 Truncated 在 node 读，
    ///  到不了 comp slice 检查。root=2/count=2 让 total_nodes=3 与实际匹配，专门触发 slice 边界。）
    #[test]
    fn read_rejects_oob_component_slice() {
        let mut bytes = two_comp_pkg_bytes();
        let ct_off = comp_table_offset(&bytes);
        // ComponentTable 条目 0：name_idx(2) + root_node_idx(4) + node_count(4) + dynamic_len(4)
        // 篡改 root_node_idx=2（comp_a 原 0），node_count=2（原 2，不变）→ end=4 > 3
        bytes[ct_off + 2..ct_off + 6].copy_from_slice(&2u32.to_le_bytes());
        // node_count 保持 2（原值），total_nodes = 2 + 1 = 3 == 实际 NodeBlock
        let err = read_package(&bytes).expect_err("oob slice should error");
        assert!(
            matches!(err, PkgError::Truncated("comp_node_slice")),
            "expected Truncated(\"comp_node_slice\"), got {err:?}"
        );
    }

    /// Important 2：malformed NodeBlock 的 parent_idx 全局值 < 组件 base → Truncated（不静默 reparent）。
    /// 构造 comp_b（base=2）的 root 节点 parent_idx=0（< 2，跨组件指向 comp_a）→ cross_comp_parent。
    #[test]
    fn read_rejects_cross_component_parent() {
        let bytes = two_comp_pkg_bytes();
        // 找 comp_b 的 root 节点在 NodeBlock 中的 parent_idx 字段位置。
        // NodeBlock 紧跟 ComponentTable（2 条目 × 14B = 28B）。
        let ct_off = comp_table_offset(&bytes);
        let nodeblock_off = ct_off + 2 * 14; // 2 组件条目
        // 节点布局：parent_idx(4) + kind(1) + style_len(4) + style_blob + text_idx(2) + src_idx(2)
        //   + class_count(2) + class_idx[] + id_idx(2) + flags(1) + tabindex(4)
        //   固定部分 = 22B + style_blob_len + 2*class_count。所有节点用默认 style → style_len 相同。
        let style_len_0 = u32::from_le_bytes(
            bytes[nodeblock_off + 5..nodeblock_off + 9].try_into().unwrap(),
        ) as usize;
        // class_count 偏移 = node_start + 9 + style_len + 4（跳过 text_idx + src_idx）
        let class_count_0 = u16::from_le_bytes(
            bytes[nodeblock_off + 9 + style_len_0 + 4..nodeblock_off + 11 + style_len_0 + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let node0_size = 22 + style_len_0 + 2 * class_count_0;
        let node1_off = nodeblock_off + node0_size;
        let style_len_1 = u32::from_le_bytes(
            bytes[node1_off + 5..node1_off + 9].try_into().unwrap(),
        ) as usize;
        let class_count_1 = u16::from_le_bytes(
            bytes[node1_off + 9 + style_len_1 + 4..node1_off + 11 + style_len_1 + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let node1_size = 22 + style_len_1 + 2 * class_count_1;
        let node2_off = nodeblock_off + node0_size + node1_size;
        // 篡改节点 2（comp_b root）的 parent_idx 从 -1 → 0（< base=2，跨组件）
        let mut patched = bytes.clone();
        patched[node2_off..node2_off + 4].copy_from_slice(&0i32.to_le_bytes());
        let err = read_package(&patched).expect_err("cross-comp parent should error");
        assert!(
            matches!(err, PkgError::Truncated("cross_comp_parent")),
            "expected Truncated(\"cross_comp_parent\"), got {err:?}"
        );
    }

    /// Important 3：两个 ComponentTable 条目指向同一 name_idx（同名组件）→ DupComponent（不静默覆盖）。
    #[test]
    fn read_rejects_duplicate_component_name() {
        let mut bytes = two_comp_pkg_bytes();
        let ct_off = comp_table_offset(&bytes);
        // ComponentTable 条目 1（comp_b）的 name_idx 改为条目 0 的 name_idx → 同名
        let name_idx_0 = u16::from_le_bytes(bytes[ct_off..ct_off + 2].try_into().unwrap());
        bytes[ct_off + 14..ct_off + 16].copy_from_slice(&name_idx_0.to_le_bytes());
        let err = read_package(&bytes).expect_err("dup component name should error");
        assert!(
            matches!(err, PkgError::DupComponent(_)),
            "expected DupComponent(_), got {err:?}"
        );
    }

    /// Minor 4：write_package 对 nodes[0].parent_idx=Some 的输入触发 debug_assert（spec 约定 nodes[0]=组件根）。
    /// write 输入由 T2/T3 控制，违反即打包器 bug；用 debug_assert（release 无代价）。
    /// 测试用 #[should_panic] 验证 debug 构建下触发。
    #[test]
    #[should_panic(expected = "nodes[0] must be root")]
    fn write_rejects_non_root_nodes_zero() {
        let mut root = tn(NodeKind::Container);
        root.parent_idx = Some(0); // 违反：nodes[0] 必须是组件根（parent=None）
        let nodes = [root];
        let rules = empty_rules();
        let input = PackageInput {
            components: vec![("c", &nodes, &rules)],
            asset_manifest: &[],
        };
        let _ = write_package(&input);
    }

    // —— T2: path 归一化 + CSS 归一化 ——

    #[test]
    fn normalize_path_strips_res_prefix() {
        assert_eq!(normalize_path("res/icons/skin.png", "res"), Some("icons/skin.png".into()));
        assert_eq!(normalize_path("./res/icons/skin.png", "res"), Some("icons/skin.png".into()));
        assert_eq!(normalize_path("res\\icons\\skin.png", "res"), Some("icons/skin.png".into()), "Win 反斜杠");
    }

    #[test]
    fn normalize_path_custom_res_dir() {
        assert_eq!(normalize_path("assets/icons/skin.png", "assets"), Some("icons/skin.png".into()));
    }

    #[test]
    fn normalize_path_outside_res_returns_none() {
        // 不在 res 目录下 → None（打包期 warning，不入 manifest）
        assert_eq!(normalize_path("other/foo.png", "res"), None);
    }

    #[test]
    fn normalize_path_rejects_false_segment_match() {
        // "pres/x" 含子串 "res/" 但 res 不是路径段 → None（边界检查）
        assert_eq!(normalize_path("pres/icons/skin.png", "res"), None, "pres/ 不是 res/ 段");
        // "ares/x" 同理
        assert_eq!(normalize_path("ares/icons/skin.png", "res"), None, "ares/ 不是 res/ 段");
    }

    #[test]
    fn normalize_path_leading_slash_res() {
        // "/res/x" — 前缀前是串首（/ 后即 res 段）→ Some
        assert_eq!(normalize_path("/res/icons/skin.png", "res"), Some("icons/skin.png".into()));
    }

    #[test]
    fn normalize_path_empty_after_strip() {
        // "res/" 剥前缀后空 → None（没有有效 path）
        assert_eq!(normalize_path("res/", "res"), None);
        assert_eq!(normalize_path("res", "res"), None, "res 无尾斜杠不构成段");
    }

    #[test]
    fn extract_component_css_merges_style_and_link() {
        // HTML 含 <style> + <link> → 合并成一个 stylesheet 串
        // 行内 style="" 由 resolve_styles 直接 bake，不进本函数产物。
        use std::io::Write;
        let tmp = std::env::temp_dir().join(format!(
            "loomgui_t2_css_{}.css",
            std::process::id()
        ));
        {
            let mut f = std::fs::File::create(&tmp).unwrap();
            f.write_all(b".b { color: blue; }").unwrap();
        }
        let href = tmp.to_string_lossy().replace('\\', "/");
        let html = format!(
            r#"<style>.a {{ color: red; }}</style><div><link rel="stylesheet" href="{href}"></div>"#
        );
        let merged = extract_component_css(&html, tmp.parent().unwrap());
        assert!(merged.contains(".a"), "merged 必含 <style> 内联规则 .a: {merged}");
        assert!(merged.contains(".b"), "merged 必含 <link> 引用文件规则 .b: {merged}");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn extract_component_css_no_style_returns_empty() {
        // 无 <style>/<link> → 空串
        let html = r#"<div class="x"><span>hi</span></div>"#;
        let merged = extract_component_css(html, std::path::Path::new("."));
        assert!(merged.is_empty(), "无 CSS 应返回空串，got: {merged}");
    }

    #[test]
    fn extract_component_css_missing_link_file_skipped() {
        // <link href> 指向不存在文件 → 跳过该 link（不 panic，<style> 仍抽）
        let html = r#"<style>.a { color: red; }</style><link rel="stylesheet" href="nope.css">"#;
        let merged = extract_component_css(html, std::path::Path::new("/nonexistent/dir"));
        assert!(merged.contains(".a"), "<style> 内联仍抽出: {merged}");
        assert!(!merged.contains("nope"), "缺失文件不进合并串: {merged}");
    }

    #[test]
    fn extract_component_css_ignores_non_stylesheet_link() {
        // <link rel="icon"> 非 stylesheet → 不抽
        let html = r#"<link rel="icon" href="favicon.ico"><style>.a { color: red; }</style>"#;
        let merged = extract_component_css(html, std::path::Path::new("."));
        assert!(merged.contains(".a"));
        assert!(!merged.contains("favicon"), "非 stylesheet 的 link 不抽: {merged}");
    }
}
