//! 包格式（.pkg.bin，当前 version=7）：Rust-internal（packager 写、runtime 读，C# 不解析）。
//!
//! 扁平布局：Header(28B) + StringTable + NodeBlock（DFS 先序，含 classes/id/flags/tabindex）+
//! AtlasSection + DynamicRuleSection（bincode 整个 DynamicRuleTable）。
//! style 字段 = bincode(ResolvedStyle)。字符串表放 text content + image src +
//! atlas filename + sprite src + classes + id_attr（统一 intern 去重）。
//! NodeBlock 末含 flags（bit0=draggable）+ tabindex:i32（None→i32::MIN）。
//! ResolvedStyle 含 transform 字段（bincode，视觉层不进 taffy）、overflow_x/overflow_y。

use crate::scene::{NodeKind, NodeId, Scene};
use crate::style::dynamic::DynamicRuleTable;
use crate::style::resolved::ResolvedStyle;

pub mod texture; // 纹理注册表（src→TexMeta）

pub const PKG_MAGIC: u32 = 0x474B504C; // 磁盘字节(LE) "LPKG"（不与 frame blob "LOOM" 撞）
pub const PKG_FORMAT_VERSION: u32 = 7; // ResolvedStyle overflow_x/y + transform 字段（bincode）
const MIN_VERSION: u32 = 7;
const MAX_VERSION: u32 = 7;
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
        let uv_min = [spr.x as f32 / aw, spr.y as f32 / ah];
        let uv_max = [(spr.x + spr.w) as f32 / aw, (spr.y + spr.h) as f32 / ah];
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

#[derive(Debug)]
pub enum PkgError {
    BadMagic,
    TooOld(u32),
    TooNew(u32),
    Truncated(&'static str),
    OobString(u16),
    Bincode(bincode::Error),
    BadKind(u8),
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

/// 序列化 Scene → .pkg.bin bytes。
///
/// 布局：Header(28B) + StringTable + NodeBlock + AtlasSection + DynamicRuleSection。
/// StringTable 收 text content + image src + atlas filename + sprite src +
/// classes + id_attr（统一 intern 去重，filename 与 image src 共用同一 stringTable 与 idx_of map）。
/// DynamicRuleSection = bincode(dynamic)（ParsedSelector + Declaration 已 Serialize/Deserialize）。
pub fn write_package(
    scene: &Scene,
    root_size: (f32, f32),
    atlas: &AtlasSection,
    dynamic: &DynamicRuleTable,
) -> Vec<u8> {
    // 1. 收 stringTable（text content + image src + atlas filename + sprite src），
    //    首次出现序建索引。filename 也走同一 stringTable（与 image src 共用 intern）。
    //    **所有 intern 必须在写 header(string_count) 之前完成**——header 的 stringCount
    //    是 StringTable 实际条数，atlas 字符串延迟到末段 intern 会破坏计数。
    let mut strings: Vec<String> = Vec::new();
    let mut idx_of: std::collections::HashMap<String, u16> = std::collections::HashMap::new();

    // 每节点：(parent_idx, kind_tag, style_blob, text_idx, src_idx)
    // scene.nodes 已是 DFS 先序、NodeId(i).0 == i（build_scene / Scene::build 不变量）。
    let mut nodes: Vec<(i32, u8, Vec<u8>, u16, u16)> = Vec::new();
    for n in &scene.nodes {
        let parent_idx = n.parent.map(|NodeId(p)| p as i32).unwrap_or(-1);
        let (kind_tag, text_idx, src_idx) = match &n.kind {
            NodeKind::Container => (KIND_CONTAINER, NULL_IDX, NULL_IDX),
            NodeKind::Button => (KIND_BUTTON, NULL_IDX, NULL_IDX),
            NodeKind::Image { src } => {
                (KIND_IMAGE, NULL_IDX, intern(src, &mut strings, &mut idx_of))
            }
            NodeKind::Text { content } => {
                (KIND_TEXT, intern(content, &mut strings, &mut idx_of), NULL_IDX)
            }
        };
        let style_blob = bincode::serialize(&n.style).expect("ResolvedStyle serializable");
        nodes.push((parent_idx, kind_tag, style_blob, text_idx, src_idx));
    }

    // 预 intern atlas 字符串（filename + sprite src），与 NodeBlock 字符串共用同一
    // stringTable 与 idx_of map。记录索引供 AtlasSection 段引用（filename 与 NodeBlock
    // image src 若同名则共用同一索引，去重）。
    let mut atlas_filename_idx: Vec<u16> = Vec::with_capacity(atlas.atlases.len());
    for a in &atlas.atlases {
        atlas_filename_idx.push(intern(&a.filename, &mut strings, &mut idx_of));
    }
    let mut sprite_src_idx: Vec<u16> = Vec::with_capacity(atlas.sprites.len());
    for s in &atlas.sprites {
        sprite_src_idx.push(intern(&s.src, &mut strings, &mut idx_of));
    }

    // 预 intern 每节点 classes + id_attr（NodeBlock 段，与 NodeBlock/atlas
    // 字符串共用同一 stringTable）。classes 每元素 intern；id_attr 若 None → NULL_IDX。
    let mut node_class_idx: Vec<Vec<u16>> = Vec::with_capacity(scene.nodes.len());
    let mut node_id_idx: Vec<u16> = Vec::with_capacity(scene.nodes.len());
    for n in &scene.nodes {
        let cls: Vec<u16> = n
            .classes
            .iter()
            .map(|c| intern(c, &mut strings, &mut idx_of))
            .collect();
        node_class_idx.push(cls);
        let id_idx = n
            .id_attr
            .as_ref()
            .map(|id| intern(id, &mut strings, &mut idx_of))
            .unwrap_or(NULL_IDX);
        node_id_idx.push(id_idx);
    }

    let mut out: Vec<u8> = Vec::new();
    // Header (28B)
    out.extend_from_slice(&PKG_MAGIC.to_le_bytes());
    out.extend_from_slice(&PKG_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags（uncompressed）
    out.extend_from_slice(&(scene.nodes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    out.extend_from_slice(&root_size.0.to_le_bytes());
    out.extend_from_slice(&root_size.1.to_le_bytes());
    // StringTable
    for s in &strings {
        let bytes = s.as_bytes();
        out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    // NodeBlock
    for (node_i, (parent_idx, kind_tag, style_blob, text_idx, src_idx)) in nodes.iter().enumerate() {
        out.extend_from_slice(&parent_idx.to_le_bytes());
        out.push(*kind_tag);
        out.extend_from_slice(&(style_blob.len() as u32).to_le_bytes());
        out.extend_from_slice(style_blob);
        out.extend_from_slice(&text_idx.to_le_bytes());
        out.extend_from_slice(&src_idx.to_le_bytes());
        // classes + id_attr（每节点，StringTable 索引）
        out.extend_from_slice(&(node_class_idx[node_i].len() as u16).to_le_bytes());
        for &cidx in &node_class_idx[node_i] {
            out.extend_from_slice(&cidx.to_le_bytes());
        }
        out.extend_from_slice(&node_id_idx[node_i].to_le_bytes());
        // flags byte（bit0=draggable）
        let flags: u8 = if scene.nodes[node_i].draggable { 0x01 } else { 0x00 };
        out.push(flags);
        // tabindex i32（None→i32::MIN 哨兵，反序列化还原 None）
        let tab = scene.nodes[node_i].tabindex.unwrap_or(i32::MIN);
        out.extend_from_slice(&tab.to_le_bytes());
    }
    // —— AtlasSection（NodeBlock 之后）——
    // atlas_count + 每 atlas{filename_idx,u16; w,u32; h,u32}
    //   + sprite_count + 每 sprite{src_idx,u16; x,y,w,h 各 u32}
    out.extend_from_slice(&(atlas.atlases.len() as u32).to_le_bytes());
    for (a, &fidx) in atlas.atlases.iter().zip(atlas_filename_idx.iter()) {
        out.extend_from_slice(&fidx.to_le_bytes());
        out.extend_from_slice(&a.width.to_le_bytes());
        out.extend_from_slice(&a.height.to_le_bytes());
    }
    out.extend_from_slice(&(atlas.sprites.len() as u32).to_le_bytes());
    for (s, &sidx) in atlas.sprites.iter().zip(sprite_src_idx.iter()) {
        out.extend_from_slice(&sidx.to_le_bytes());
        out.extend_from_slice(&s.x.to_le_bytes());
        out.extend_from_slice(&s.y.to_le_bytes());
        out.extend_from_slice(&s.w.to_le_bytes());
        out.extend_from_slice(&s.h.to_le_bytes());
    }
    // —— DynamicRuleSection（AtlasSection 之后）——
    // bincode 整个 DynamicRuleTable（含 ParsedSelector + Declarations，均已 Serialize/Deserialize）。
    let dynamic_blob = bincode::serialize(dynamic).expect("DynamicRuleTable serializable");
    out.extend_from_slice(&(dynamic_blob.len() as u32).to_le_bytes());
    out.extend_from_slice(&dynamic_blob);
    out
}

/// 反序列化 .pkg.bin → (Scene, root_size, AtlasSection)（含版本协商）。
/// NodeBlock 含 classes/id_attr；末段 DynamicRuleSection 填 Scene.dynamic_rules。
/// 返回元组不变（dynamic 填进 Scene，不外露）。
pub fn read_package(bytes: &[u8]) -> Result<(Scene, (f32, f32), AtlasSection), PkgError> {
    let mut r = Reader::new(bytes);
    // Header
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
    let node_count = r.u32("node_count")? as usize;
    let string_count = r.u32("string_count")? as usize;
    let root_w = r.f32("root_w")?;
    let root_h = r.f32("root_h")?;
    // StringTable
    let mut strings: Vec<String> = Vec::with_capacity(string_count);
    for _ in 0..string_count {
        let len = r.u16("str_len")? as usize;
        let s = r.utf8(len, "str_bytes")?;
        strings.push(s);
    }
    // NodeBlock → entries（tabindex 来自 flags 后 i32，i32::MIN→None）
    let mut entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let pidx = r.i32("parent_idx")?;
        let kind_tag = r.u8("kind")?;
        let style_len = r.u32("style_len")? as usize;
        let style: ResolvedStyle = bincode::deserialize(r.take(style_len, "style_blob")?)?;
        let text_idx = r.u16("text_idx")?;
        let src_idx = r.u16("src_idx")?;
        // classes + id_attr
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
        // flags byte（bit0=draggable）
        let flags = r.u8("flags")?;
        let draggable = (flags & 0x01) != 0;
        // tabindex i32（i32::MIN 哨兵 → None）
        let tab_raw = r.i32("tabindex")?;
        let tabindex = if tab_raw == i32::MIN { None } else { Some(tab_raw) };
        let parent = if pidx < 0 { None } else { Some(pidx as usize) };
        let kind = match kind_tag {
            KIND_CONTAINER => NodeKind::Container,
            KIND_BUTTON => NodeKind::Button,
            KIND_IMAGE => NodeKind::Image {
                src: string_at(&strings, src_idx)?,
            },
            KIND_TEXT => NodeKind::Text {
                content: string_at(&strings, text_idx)?,
            },
            other => return Err(PkgError::BadKind(other)),
        };
        entries.push((parent, kind, style, classes, id_attr, draggable, tabindex));
    }
    // —— AtlasSection ——
    let atlas_count = r.u32("atlas_count")? as usize;
    let mut atlases: Vec<AtlasInfo> = Vec::with_capacity(atlas_count);
    for _ in 0..atlas_count {
        let fidx = r.u16("atlas_filename_idx")?;
        let w = r.u32("atlas_w")?;
        let h = r.u32("atlas_h")?;
        atlases.push(AtlasInfo {
            filename: string_at(&strings, fidx)?,
            width: w,
            height: h,
        });
    }
    let sprite_count = r.u32("sprite_count")? as usize;
    let mut sprites: Vec<AtlasSprite> = Vec::with_capacity(sprite_count);
    for _ in 0..sprite_count {
        let sidx = r.u16("sprite_src_idx")?;
        let x = r.u32("sprite_x")?;
        let y = r.u32("sprite_y")?;
        let w = r.u32("sprite_w")?;
        let h = r.u32("sprite_h")?;
        sprites.push(AtlasSprite {
            src: string_at(&strings, sidx)?,
            x,
            y,
            w,
            h,
        });
    }
    // —— DynamicRuleSection ——
    let dynamic_len = r.u32("dynamic_len")? as usize;
    let dynamic: DynamicRuleTable = bincode::deserialize(r.take(dynamic_len, "dynamic_blob")?)?;
    let mut scene = Scene::build(&entries);
    scene.dynamic_rules = dynamic;
    Ok((scene, (root_w, root_h), AtlasSection { atlases, sprites }))
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
    fn f32(&mut self, ctx: &'static str) -> Result<f32, PkgError> {
        Ok(f32::from_le_bytes(
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

    #[test]
    fn write_read_roundtrip_preserves_scene() {
        // 手搓一个覆盖 4 种 kind + 嵌套的 Scene（不走 parse，靠 Scene::build）。
        let mut img_style = ResolvedStyle::default();
        img_style.background_color = Some([1.0, 0.0, 0.0, 1.0]);
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (
                Some(0),
                NodeKind::Text {
                    content: "hi".into(),
                },
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
            ),
            (
                Some(0),
                NodeKind::Image {
                    src: "logo.png".into(),
                },
                img_style.clone(),
                Vec::new(),
                None,
                false,
                None,
            ),
            (None, NodeKind::Button, ResolvedStyle::default(), Vec::new(), None, false, None),
        ];
        let scene = Scene::build(&entries);

        let bytes = write_package(&scene, (1080.0, 1920.0), &AtlasSection::default(), &crate::style::dynamic::DynamicRuleTable::default());
        let (scene2, rs, _atlas) = read_package(&bytes).expect("read ok");

        assert_eq!(rs, (1080.0, 1920.0));
        assert_eq!(scene2.nodes.len(), scene.nodes.len());
        // 结构：parent / children
        for (a, b) in scene.nodes.iter().zip(scene2.nodes.iter()) {
            assert_eq!(a.parent, b.parent);
            assert_eq!(a.children, b.children);
        }
        // kind + payload
        assert!(matches!(&scene2.nodes[1].kind, NodeKind::Text { content } if content == "hi"));
        assert!(matches!(&scene2.nodes[2].kind, NodeKind::Image { src } if src == "logo.png"));
        assert!(matches!(scene2.nodes[0].kind, NodeKind::Container));
        assert!(matches!(scene2.nodes[3].kind, NodeKind::Button));
        // style 经 bincode round-trip（background_color 非 None）——全字段相等
        assert_eq!(scene2.nodes[2].style, img_style);
        // 其他节点 style 也应 round-trip（default）
        assert_eq!(scene2.nodes[0].style, scene.nodes[0].style);
        assert_eq!(scene2.nodes[1].style, scene.nodes[1].style);
        assert_eq!(scene2.nodes[3].style, scene.nodes[3].style);
    }

    #[test]
    fn read_rejects_bad_magic() {
        let mut bad = vec![0u8; 28];
        // magic 改成 "LOOM"（frame blob 的）→ 应被拒
        bad[0..4].copy_from_slice(&0x4D4F4F4Cu32.to_le_bytes());
        assert!(matches!(read_package(&bad), Err(PkgError::BadMagic)));
    }

    #[test]
    fn read_rejects_unsupported_version() {
        // 借 round-trip 测的合法包（v7），把 version 字段（offset 4）改成 8 / 5。
        // MIN_VERSION=MAX_VERSION=7：v5/v6 → TooOld，v8 → TooNew。
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> =
            vec![(None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None)];
        let mut bytes = write_package(&Scene::build(&entries), (100.0, 100.0), &AtlasSection::default(), &crate::style::dynamic::DynamicRuleTable::default());
        bytes[4..8].copy_from_slice(&8u32.to_le_bytes()); // version=8 → too new
        assert!(matches!(read_package(&bytes), Err(PkgError::TooNew(8))));
        bytes[4..8].copy_from_slice(&5u32.to_le_bytes()); // version=5 → too old（v7 起拒 v6 及以下）
        assert!(matches!(read_package(&bytes), Err(PkgError::TooOld(5))));
    }

    #[test]
    fn atlas_section_round_trips() {
        // 空 scene + 非空 AtlasSection → round-trip 后 atlas 字段逐项相等。
        // 覆盖：filename intern、sprite src intern（与 NodeBlock 共 stringTable）、
        // u32 × 4 sprite 字段、atlas_count/sprite_count。
        let entries = vec![(None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None)];
        let scene = Scene::build(&entries);
        let atlas = AtlasSection {
            atlases: vec![AtlasInfo { filename: "a.atlas.png".into(), width: 512, height: 256 }],
            sprites: vec![
                AtlasSprite { src: "x.png".into(), x: 0, y: 0, w: 64, h: 32 },
                AtlasSprite { src: "y.png".into(), x: 64, y: 0, w: 100, h: 200 },
            ],
        };
        let bytes = write_package(&scene, (10.0, 10.0), &atlas, &crate::style::dynamic::DynamicRuleTable::default());
        let (_s, _rs, a2) = read_package(&bytes).unwrap();
        assert_eq!(a2, atlas);
    }

    #[test]
    fn build_registry_maps_sprites_to_atlas_zero_tex_id_one() {
        // 单图集契约：atlas[0] → tex_id=1，sprite UV = (x,y,w,h)/atlas_size。
        // build_registry 是 load_package 的核心步骤，独立测锁住其映射。
        let section = AtlasSection {
            atlases: vec![AtlasInfo { filename: "a.png".into(), width: 512, height: 256 }],
            sprites: vec![
                AtlasSprite { src: "s1.png".into(), x: 0, y: 0, w: 64, h: 32 },
                AtlasSprite { src: "s2.png".into(), x: 64, y: 32, w: 100, h: 200 },
            ],
        };
        let reg = build_registry(&section);
        let m1 = reg.get("s1.png").expect("s1 registered");
        assert_eq!(m1.tex_id, 1, "atlas[0] → tex_id 1");
        assert_eq!(m1.uv_min, [0.0, 0.0]);
        assert_eq!(m1.uv_max, [64.0 / 512.0, 32.0 / 256.0]);
        assert_eq!((m1.width, m1.height), (64, 32));
        let m2 = reg.get("s2.png").expect("s2 registered");
        assert_eq!(m2.tex_id, 1, "同图集 sprite 共享 tex_id");
        assert_eq!(m2.uv_min, [64.0 / 512.0, 32.0 / 256.0]);
        assert_eq!(m2.uv_max, [(64.0 + 100.0) / 512.0, (32.0 + 200.0) / 256.0]);
    }

    #[test]
    fn build_registry_empty_atlases_returns_empty() {
        // 无 atlas（inline / 空 package）→ 空 registry（measure Image 走 64×64 兜底）。
        let section = AtlasSection::default();
        let reg = build_registry(&section);
        assert!(reg.is_empty());
    }

    #[test]
    fn pkg_v3_round_trip_preserves_dynamic_rules() {
        use crate::parse::css::Declaration;
        use crate::parse::selector::parse_selector;
        use crate::style::dynamic::{DynamicRule, DynamicRuleTable};
        use crate::style::resolved::ResolvedStyle;
        let entries = vec![(
            None,
            NodeKind::Container,
            ResolvedStyle::default(),
            Vec::new(),
            None,
            false,
            None,
        )];
        let scene = Scene::build(&entries);
        let dynamic = DynamicRuleTable {
            rules: vec![DynamicRule {
                selector: parse_selector(".btn:hover").unwrap(),
                declarations: vec![Declaration {
                    prop: "background-color".to_string(),
                    value: "#0000ff".to_string(),
                }],
            }],
        };
        let pkg = write_package(
            &scene,
            (100.0, 50.0),
            &AtlasSection::default(),
            &dynamic,
        );
        let (scene2, _rs, _atlas) = read_package(&pkg).unwrap();
        assert_eq!(scene2.dynamic_rules.rules.len(), 1);
        assert!(scene2.dynamic_rules.rules[0].selector.compound[0].pseudo_hover);
        assert_eq!(
            scene2.dynamic_rules.rules[0].declarations[0].prop,
            "background-color"
        );
    }

    #[test]
    fn pkg_v3_rejects_v2() {
        // 手搓 v2 包（version=2）——应被拒 TooOld
        let mut pkg = Vec::new();
        pkg.extend_from_slice(&PKG_MAGIC.to_le_bytes());
        pkg.extend_from_slice(&2u32.to_le_bytes()); // version=2
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&100.0f32.to_le_bytes());
        pkg.extend_from_slice(&50.0f32.to_le_bytes());
        match read_package(&pkg) {
            Err(PkgError::TooOld(2)) => (), // 预期
            other => panic!("v2 应被拒，got {:?}", other),
        }
    }

    #[test]
    fn pkg_v3_empty_dynamic_rules() {
        use crate::style::dynamic::DynamicRuleTable;
        use crate::style::resolved::ResolvedStyle;
        let entries = vec![(
            None,
            NodeKind::Container,
            ResolvedStyle::default(),
            Vec::new(),
            None,
            false,
            None,
        )];
        let scene = Scene::build(&entries);
        let pkg = write_package(
            &scene,
            (100.0, 50.0),
            &AtlasSection::default(),
            &DynamicRuleTable::default(),
        );
        let (scene2, _, _) = read_package(&pkg).unwrap();
        assert!(
            scene2.dynamic_rules.rules.is_empty(),
            "空 dynamic → rules 空"
        );
    }

    #[test]
    fn pkg_v3_nodeblock_preserves_classes_and_id_attr() {
        use crate::style::resolved::ResolvedStyle;
        let entries = vec![(
            None,
            NodeKind::Container,
            ResolvedStyle::default(),
            vec!["a".to_string(), "b".to_string()],
            Some("x".to_string()),
            false,
            None,
        )];
        let scene = Scene::build(&entries);
        let pkg = write_package(
            &scene,
            (100.0, 50.0),
            &AtlasSection::default(),
            &crate::style::dynamic::DynamicRuleTable::default(),
        );
        let (scene2, _, _) = read_package(&pkg).unwrap();
        assert_eq!(
            scene2.nodes[0].classes,
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(scene2.nodes[0].id_attr.as_deref(), Some("x"));
    }

    #[test]
    fn stringtable_dedups_repeated_strings() {
        // 两个 Text 同 content → stringTable 只一条，textIdx 相同。
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (
                Some(0),
                NodeKind::Text {
                    content: "dup".into(),
                },
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
            ),
            (
                Some(0),
                NodeKind::Text {
                    content: "dup".into(),
                },
                ResolvedStyle::default(),
                Vec::new(),
                None,
                false,
                None,
            ),
        ];
        let bytes = write_package(&Scene::build(&entries), (10.0, 10.0), &AtlasSection::default(), &crate::style::dynamic::DynamicRuleTable::default());
        // stringCount（offset 16）应为 1（"dup" 去重）
        let sc = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        assert_eq!(sc, 1, "重复 content 应去重为 1 条");
        let (scene2, _, _) = read_package(&bytes).unwrap();
        assert!(matches!(&scene2.nodes[1].kind, NodeKind::Text { content } if content == "dup"));
        assert!(matches!(&scene2.nodes[2].kind, NodeKind::Text { content } if content == "dup"));
    }

    #[test]
    fn pkg_v4_preserves_draggable_flag() {
        use crate::style::resolved::ResolvedStyle;
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), Vec::new(), None, true, None),
        ];
        let scene = Scene::build(&entries);
        let pkg = write_package(&scene, (100.0, 50.0), &AtlasSection::default(), &crate::style::dynamic::DynamicRuleTable::default());
        let (scene2, _, _) = read_package(&pkg).unwrap();
        assert!(!scene2.nodes[0].draggable, "root draggable=false round-trip");
        assert!(scene2.nodes[1].draggable, "btn draggable=true round-trip");
    }

    #[test]
    fn pkg_v4_rejects_v3() {
        // 手搓 v3 包（version=3）——应被拒 TooOld
        let mut pkg = Vec::new();
        pkg.extend_from_slice(&PKG_MAGIC.to_le_bytes());
        pkg.extend_from_slice(&3u32.to_le_bytes()); // version=3
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&100.0f32.to_le_bytes());
        pkg.extend_from_slice(&50.0f32.to_le_bytes());
        match read_package(&pkg) {
            Err(PkgError::TooOld(3)) => (), // 预期
            other => panic!("v3 应被拒，got {:?}", other),
        }
    }

    #[test]
    fn pkg_v5_preserves_tabindex() {
        use crate::style::resolved::ResolvedStyle;
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle, Vec<String>, Option<String>, bool, Option<i32>)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default(), Vec::new(), None, false, None),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), Vec::new(), None, false, Some(0)),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), Vec::new(), None, false, Some(3)),
            (Some(0), NodeKind::Button, ResolvedStyle::default(), Vec::new(), None, false, Some(-1)),
        ];
        let scene = Scene::build(&entries);
        let pkg = write_package(&scene, (100.0, 50.0), &AtlasSection::default(), &crate::style::dynamic::DynamicRuleTable::default());
        let (scene2, _, _) = read_package(&pkg).unwrap();
        assert_eq!(scene2.nodes[0].tabindex, None, "root tabindex=None round-trip");
        assert_eq!(scene2.nodes[1].tabindex, Some(0));
        assert_eq!(scene2.nodes[2].tabindex, Some(3));
        assert_eq!(scene2.nodes[3].tabindex, Some(-1));
    }

    #[test]
    fn pkg_v6_rejects_v5() {
        // 手搓 v5 包（version=5）——应被拒 TooOld
        let mut pkg = Vec::new();
        pkg.extend_from_slice(&PKG_MAGIC.to_le_bytes());
        pkg.extend_from_slice(&5u32.to_le_bytes()); // version=5
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&0u32.to_le_bytes());
        pkg.extend_from_slice(&100.0f32.to_le_bytes());
        pkg.extend_from_slice(&50.0f32.to_le_bytes());
        match read_package(&pkg) {
            Err(PkgError::TooOld(5)) => (), // 预期
            other => panic!("v5 应被拒，got {:?}", other),
        }
    }
}
