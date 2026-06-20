//! 包格式（spec §5）：.pkg.bin v1。Rust-internal（packager 写、runtime 读，C# 不解析）。
//!
//! 扁平布局：Header(28B) + StringTable + NodeBlock（DFS 先序）。style 字段 = bincode(ResolvedStyle)。
//! 字符串表只放 text content + image src（font_family 随 style blob）。

use crate::scene::{NodeKind, NodeId, Scene};
use crate::style::resolved::ResolvedStyle;

pub mod texture; // v1b.2：纹理注册表（src→TexMeta）

pub const PKG_MAGIC: u32 = 0x474B504C; // 磁盘字节(LE) "LPKG"（不与 frame blob "LOOM" 撞）
pub const PKG_FORMAT_VERSION: u32 = 1;
const MIN_VERSION: u32 = 1;
const MAX_VERSION: u32 = 1;
const NULL_IDX: u16 = 0xFFFF;

const KIND_CONTAINER: u8 = 0;
const KIND_BUTTON: u8 = 1;
const KIND_IMAGE: u8 = 2;
const KIND_TEXT: u8 = 3;

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

/// 序列化 Scene → .pkg.bin bytes（spec §5）。
pub fn write_package(scene: &Scene, root_size: (f32, f32)) -> Vec<u8> {
    // 1. 收 stringTable（text content + image src），首次出现序建索引。
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
    out
}

/// 反序列化 .pkg.bin → (Scene, root_size)（spec §5 + §6 版本协商）。
pub fn read_package(bytes: &[u8]) -> Result<(Scene, (f32, f32)), PkgError> {
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
    let scene = Scene::build(&entries);
    Ok((scene, (root_w, root_h)))
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

        let bytes = write_package(&scene, (1080.0, 1920.0));
        let (scene2, rs) = read_package(&bytes).expect("read ok");

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
        // 借 round-trip 测的合法包，把 version 字段（offset 4）改成 2 / 0
        let entries: Vec<(Option<usize>, NodeKind, ResolvedStyle)> =
            vec![(None, NodeKind::Container, ResolvedStyle::default())];
        let mut bytes = write_package(&Scene::build(&entries), (100.0, 100.0));
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes()); // version=2 → too new
        assert!(matches!(read_package(&bytes), Err(PkgError::TooNew(2))));
        bytes[4..8].copy_from_slice(&0u32.to_le_bytes()); // version=0 → too old
        assert!(matches!(read_package(&bytes), Err(PkgError::TooOld(0))));
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
        let bytes = write_package(&Scene::build(&entries), (10.0, 10.0));
        // stringCount（offset 16）应为 1（"dup" 去重）
        let sc = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        assert_eq!(sc, 1, "重复 content 应去重为 1 条");
        let (scene2, _) = read_package(&bytes).unwrap();
        assert!(matches!(&scene2.nodes[1].kind, NodeKind::Text { content } if content == "dup"));
        assert!(matches!(&scene2.nodes[2].kind, NodeKind::Text { content } if content == "dup"));
    }
}
