//! 包格式（spec §5）：.pkg.bin v2。Rust-internal（packager 写、runtime 读，C# 不解析）。
//!
//! 扁平布局：Header(28B) + StringTable + NodeBlock（DFS 先序）+ AtlasSection（v2 新增）。
//! style 字段 = bincode(ResolvedStyle)。字符串表放 text content + image src +
//! atlas filename + sprite src（统一 intern 去重，font_family 随 style blob）。

use crate::scene::{NodeKind, NodeId, Scene};
use crate::style::resolved::ResolvedStyle;

pub mod texture; // v1b.2：纹理注册表（src→TexMeta）

pub const PKG_MAGIC: u32 = 0x474B504C; // 磁盘字节(LE) "LPKG"（不与 frame blob "LOOM" 撞）
pub const PKG_FORMAT_VERSION: u32 = 2; // v2：加 AtlasSection（v1=纯 scene）
const MIN_VERSION: u32 = 2;
const MAX_VERSION: u32 = 2;
const NULL_IDX: u16 = 0xFFFF;

const KIND_CONTAINER: u8 = 0;
const KIND_BUTTON: u8 = 1;
const KIND_IMAGE: u8 = 2;
const KIND_TEXT: u8 = 3;

/// atlas 元数据（.pkg.bin v2 AtlasSection 的内存形）。
/// 甲-B 单图集：atlas.len()≤1，所有 sprite 属 atlas[0]（build_registry 硬编码 tex_id=1）。
/// 多图集（v1b.4+）需 sprite 带 atlas_idx 才能分流——当前模型不支持。
#[derive(Debug, Clone, PartialEq)]
pub struct AtlasSection {
    pub atlases: Vec<AtlasInfo>,   // 甲-B len=1
    pub sprites: Vec<AtlasSprite>, // 全部 sprite（甲-B 都属 atlas 0）
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

/// 从 AtlasSection 建 TextureView registry。甲-B 单图集：atlas[0] 得 tex_id=1，
/// 所有 sprite 属它。空 atlas（inline / 空 package）→ 空 registry。
/// （多图集 v1b.4+ 需 sprite 带 atlas_idx 才能分流。）
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

/// 序列化 Scene → .pkg.bin bytes（spec §5；v2 = +AtlasSection 末段）。
///
/// 布局：Header(28B) + StringTable + NodeBlock + AtlasSection。
/// StringTable 收 text content + image src + atlas filename + sprite src（统一 intern
/// 去重，filename 与 image src 复用同一 stringTable 与 idx_of map）。
pub fn write_package(scene: &Scene, root_size: (f32, f32), atlas: &AtlasSection) -> Vec<u8> {
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

    let mut out: Vec<u8> = Vec::new();
    // Header (28B)
    out.extend_from_slice(&PKG_MAGIC.to_le_bytes());
    out.extend_from_slice(&PKG_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags（v1 uncompressed）
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
    for (parent_idx, kind_tag, style_blob, text_idx, src_idx) in &nodes {
        out.extend_from_slice(&parent_idx.to_le_bytes());
        out.push(*kind_tag);
        out.extend_from_slice(&(style_blob.len() as u32).to_le_bytes());
        out.extend_from_slice(style_blob);
        out.extend_from_slice(&text_idx.to_le_bytes());
        out.extend_from_slice(&src_idx.to_le_bytes());
    }
    // —— AtlasSection（v2 新增，NodeBlock 之后）——
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
    out
}

/// 反序列化 .pkg.bin → (Scene, root_size, AtlasSection)（spec §5 + §6 版本协商）。
/// v2：NodeBlock 之后追加 AtlasSection。
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
    // NodeBlock → entries
    let mut entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let pidx = r.i32("parent_idx")?;
        let kind_tag = r.u8("kind")?;
        let style_len = r.u32("style_len")? as usize;
        let style: ResolvedStyle = bincode::deserialize(r.take(style_len, "style_blob")?)?;
        let text_idx = r.u16("text_idx")?;
        let src_idx = r.u16("src_idx")?;
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
        entries.push((parent, kind, style));
    }
    // —— AtlasSection（v2）——
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
    let scene = Scene::build(&entries);
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
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default()),
            (
                Some(0),
                NodeKind::Text {
                    content: "hi".into(),
                },
                ResolvedStyle::default(),
            ),
            (
                Some(0),
                NodeKind::Image {
                    src: "logo.png".into(),
                },
                img_style.clone(),
            ),
            (None, NodeKind::Button, ResolvedStyle::default()),
        ];
        let scene = Scene::build(&entries);

        let bytes = write_package(&scene, (1080.0, 1920.0), &AtlasSection::default());
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
        // 借 round-trip 测的合法包（v2），把 version 字段（offset 4）改成 3 / 1。
        // MIN_VERSION=MAX_VERSION=2：v1 → TooOld，v3 → TooNew。
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> =
            vec![(None, NodeKind::Container, ResolvedStyle::default())];
        let mut bytes = write_package(&Scene::build(&entries), (100.0, 100.0), &AtlasSection::default());
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes()); // version=3 → too new
        assert!(matches!(read_package(&bytes), Err(PkgError::TooNew(3))));
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes()); // version=1 → too old
        assert!(matches!(read_package(&bytes), Err(PkgError::TooOld(1))));
    }

    #[test]
    fn atlas_section_round_trips() {
        // 空 scene + 非空 AtlasSection → round-trip 后 atlas 字段逐项相等。
        // 覆盖：filename intern、sprite src intern（与 NodeBlock 共 stringTable）、
        // u32 × 4 sprite 字段、atlas_count/sprite_count。
        let entries = vec![(None, NodeKind::Container, ResolvedStyle::default())];
        let scene = Scene::build(&entries);
        let atlas = AtlasSection {
            atlases: vec![AtlasInfo { filename: "a.atlas.png".into(), width: 512, height: 256 }],
            sprites: vec![
                AtlasSprite { src: "x.png".into(), x: 0, y: 0, w: 64, h: 32 },
                AtlasSprite { src: "y.png".into(), x: 64, y: 0, w: 100, h: 200 },
            ],
        };
        let bytes = write_package(&scene, (10.0, 10.0), &atlas);
        let (_s, _rs, a2) = read_package(&bytes).unwrap();
        assert_eq!(a2, atlas);
    }

    #[test]
    fn build_registry_maps_sprites_to_atlas_zero_tex_id_one() {
        // 甲-B 单图集契约：atlas[0] → tex_id=1，sprite UV = (x,y,w,h)/atlas_size。
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
    fn stringtable_dedups_repeated_strings() {
        // 两个 Text 同 content → stringTable 只一条，textIdx 相同。
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> = vec![
            (None, NodeKind::Container, ResolvedStyle::default()),
            (
                Some(0),
                NodeKind::Text {
                    content: "dup".into(),
                },
                ResolvedStyle::default(),
            ),
            (
                Some(0),
                NodeKind::Text {
                    content: "dup".into(),
                },
                ResolvedStyle::default(),
            ),
        ];
        let bytes = write_package(&Scene::build(&entries), (10.0, 10.0), &AtlasSection::default());
        // stringCount（offset 16）应为 1（"dup" 去重）
        let sc = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        assert_eq!(sc, 1, "重复 content 应去重为 1 条");
        let (scene2, _, _) = read_package(&bytes).unwrap();
        assert!(matches!(&scene2.nodes[1].kind, NodeKind::Text { content } if content == "dup"));
        assert!(matches!(&scene2.nodes[2].kind, NodeKind::Text { content } if content == "dup"));
    }
}
