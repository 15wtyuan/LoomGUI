//! Text 层：给定文本 + 字体 + 约束宽，产出 TextLayout（SOA 三表 glyphs/runs/lines）。
//!
//! 对应主文档 §9。v0 实现要点：
//! - 字体度量走 ttf-parser（ttf-parser 0.20 API，见下方适配注释）。
//! - 断行用贪心按空白 + 宽度约束（unicode-linebreak 留作 v1.x 严格换行）。
//! - glyph 存绝对坐标（已累加 advance + 已应用 align 偏移），后端拼 quad 零累加（§9.2 核心）。

use serde::Serialize;
use ttf_parser::Face;

/// 单个字形。坐标为绝对坐标（pen 位 = glyph.x/y + bearing）。
#[derive(Debug, Clone, Serialize)]
pub struct Glyph {
    pub glyph_id: u16,
    /// Unicode 码点（v1a Phase 2 新增：Unity `Font.GetCharacterInfo(char)` 按码点查，
    /// 非 ttf glyph_id）。`measure_text` 遍历 `content.chars()` 时 `c as u32` 填入。
    pub codepoint: u32,
    /// pen x（已累加 advance + 已应用 align 偏移）。
    pub x: f32,
    /// 行内 pen y（= line_y，未加 baseline）。
    pub y: f32,
    /// pen 位 → 字形 quad 左上的 x 偏移（来自 glyph bbox x_min）。
    pub bearing_x: f32,
    /// pen 位 → 字形 quad 左上的 y 偏移（来自 glyph bbox y_max，顶到 baseline）。
    pub bearing_y: f32,
}

/// 单 run：一组连续字形。v0 单字体单 run，glyphs 直接内联。
#[derive(Debug, Clone, Serialize)]
pub struct GlyphRun {
    pub font_size: f32,
    pub glyphs: Vec<Glyph>,
}

/// 一行文本。
#[derive(Debug, Clone, Serialize)]
pub struct Line {
    /// 行顶 y（相对布局原点）。
    pub y: f32,
    /// 行高（line-height 已烤进，后端不重套；§9.1）。
    pub height: f32,
    /// 行 baseline（绝对 y）。
    pub baseline: f32,
    /// 行内文字宽度。
    pub width: f32,
    pub runs: Vec<GlyphRun>,
}

/// 文本布局结果（SOA 三表：lines/runs/glyphs）。
#[derive(Debug, Clone, Serialize)]
pub struct TextLayout {
    pub text_width: f32,
    pub text_height: f32,
    pub lines: Vec<Line>,
}

/// 封装一个 ttf 字体。v0 单字体无 fallback。
///
/// Face 借用 bytes；用 `Box::leak` 拿 `'static` 切片满足生命周期。
/// 这是 v0 单字体的简化做法——leak 的内存不释放，进程级单字体可接受。
pub struct Font {
    pub face: Face<'static>,
    // 持有字体字节；face 实际借用的是 leaked 副本（见 from_bytes）。
    // 保留原 bytes 仅为完整性，不参与生命周期（leaked 切片才真正存活）。
    _bytes: Vec<u8>,
}

impl Font {
    pub fn from_path(path: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        // ttf-parser Face 借用 bytes；用 Box::leak 拿 'static 切片（v0 单字体够用）。
        let leaked: &'static [u8] = Box::leak(bytes.clone().into_boxed_slice());
        let face = Face::parse(leaked, 0).map_err(|e| format!("{:?}", e))?;
        Ok(Font {
            face,
            _bytes: bytes,
        })
    }

    pub fn ascent(&self, font_size: f32) -> f32 {
        let asc = self.face.ascender() as f32;
        let units = self.face.units_per_em() as f32;
        asc / units * font_size
    }

    /// 字体下降量，负值。
    pub fn descent(&self, font_size: f32) -> f32 {
        let desc = self.face.descender() as f32;
        let units = self.face.units_per_em() as f32;
        desc / units * font_size
    }

    pub fn line_gap(&self, font_size: f32) -> f32 {
        let lg = self.face.line_gap() as f32;
        let units = self.face.units_per_em() as f32;
        lg / units * font_size
    }
}

/// 测量并布局文本。
///
/// - `line_height`：倍数，`0.0` = normal（= ascent - descent + line_gap）。
/// - `max_width`：`None` 表示不换行；`nowrap=true` 时强制单行（white-space:nowrap）。
///
/// # ttf-parser 0.20 API 适配（与 brief 推测的差异）
/// - `glyph_advance_width(GlyphId) -> Option<i16>` 在 0.20 不存在；
///   用 `glyph_hor_advance(GlyphId) -> Option<u16>`（注意返回 u16）。
/// - `kerning_for(GlyphId, GlyphId) -> Option<i16>` 在 0.20 的 Face 上不直接暴露；
///   通过 `face.tables().kern.as_ref().and_then(|k| k.glyphs_kerning(l, r))`。
/// - `glyph_index(ch) -> Option<GlyphId>`、`GlyphId(pub u16)`、`glyph_bounding_box(GlyphId)`
///   返回 `Rect{x_min,y_min,x_max,y_max}`、`ascender/descender/line_gap/units_per_em`、
///   `Face::parse(bytes, 0)` 均与 brief 一致。
//
// 参数清单是 brief verbatim 契约（下游 Task 6 layout 的 MeasureFunc 消费），
// 不为 clippy 折叠——见 docs 主文档 §9。
#[allow(clippy::too_many_arguments)]
pub fn measure_text(
    content: &str,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    align: crate::style::resolved::TextAlign,
    nowrap: bool,
    max_width: Option<f32>,
    font: &Font,
) -> TextLayout {
    let ascent = font.ascent(font_size);
    let descent = font.descent(font_size); // 负
    let line_gap = font.line_gap(font_size);
    let units = font.face.units_per_em() as f32;

    // Line.height：line-height 生效则烤进 height（后端不重套，§9.1）；
    // 否则用字体自然行高（ascent - descent + line_gap）。
    let line_h = if line_height > 0.0 {
        font_size * line_height
    } else {
        ascent - descent + line_gap
    };
    // baseline：v0 简化占位——实现期对照 Chrome 调（§9.1）。
    let baseline = if line_height > 0.0 {
        (line_h + ascent - descent) / 2.0 - descent.abs()
    } else {
        ascent
    };

    // 单位换算辅助：设计单位 → px。
    let to_px = |design: f32| -> f32 { design / units * font_size };

    // 字距 + advance 度量。
    //
    // ttf-parser 0.20：Face 不直接暴露字距；kern 表是多个子表的集合，
    // 需遍历水平子表逐个查询（Subtable::glyphs_kerning）。
    let kerning = |left: ttf_parser::GlyphId, right: ttf_parser::GlyphId| -> Option<i16> {
        let table = font.face.tables().kern.as_ref()?;
        // kern::Table.subtables 是 Subtables（实现 IntoIterator）；按值迭代副本。
        for sub in table.subtables.into_iter() {
            if !sub.horizontal || sub.has_state_machine {
                continue;
            }
            if let Some(v) = sub.glyphs_kerning(left, right) {
                return Some(v);
            }
        }
        None
    };
    let advance = |gid: ttf_parser::GlyphId| -> f32 {
        font.face
            .glyph_hor_advance(gid)
            .map(|v| to_px(v as f32))
            .unwrap_or(0.0)
    };

    // 度量一段文本的宽度（含字距）。
    let measure_width = |s: &str| -> f32 {
        let mut pen = 0.0f32;
        let mut prev: Option<ttf_parser::GlyphId> = None;
        for ch in s.chars() {
            let gid = font.face.glyph_index(ch).unwrap_or_default();
            if let Some(p) = prev {
                if let Some(k) = kerning(p, gid) {
                    pen += to_px(k as f32);
                }
            }
            pen += advance(gid) + letter_spacing;
            prev = Some(gid);
        }
        pen
    };

    // 断行：unicode-linebreak UAX#14 换行机会 + 贪心填行（§9.1 CJK 逐字）。
    // 替换 v0 的 split(' ') 贪心（对 CJK 完全失效——中文无空格 → 整段一 word 无法换行）。
    // white-space:nowrap 强制单行。
    //
    // unicode-linebreak 0.1.5 实测 API（已核对 crates 源码，与 brief 草稿有出入）：
    // - `linebreaks(s)` 返回 `impl Iterator<Item=(usize, BreakOpportunity)>`（非 Vec）；
    // - 枚举名是 `BreakOpportunity`（非 brief 的 BreakType），变体 `Mandatory`/`Allowed`；
    // - offset 语义 = "断点之后字符的字节序号"，即前段 = content[..offset]，后段 = content[offset..]。
    use unicode_linebreak::{linebreaks, BreakOpportunity};
    let max_w = max_width.unwrap_or(f32::MAX);

    // 1. 取所有 break opportunities（byte offset + 类型），收成 Vec 便于多轮迭代。
    let opportunities: Vec<(usize, BreakOpportunity)> = linebreaks(content).collect();

    // 2. 切 segments：相邻 break 之间的文本片段。unicode-linebreak 在空白后断，
    //    segment 自含尾空白 → 行首无多余空格（删 v0 的 space_w 重加逻辑）。
    let mut segments: Vec<(&str, BreakOpportunity)> = Vec::new();
    let mut prev = 0usize;
    for &(offset, btype) in &opportunities {
        if offset > prev {
            segments.push((&content[prev..offset], btype));
        }
        prev = offset;
    }
    if prev < content.len() {
        segments.push((&content[prev..], BreakOpportunity::Allowed));
    }

    // 3. 贪心填行。
    let mut lines: Vec<(String, f32)> = Vec::new(); // (text, width)
    let mut cur = String::new();
    let mut cur_w = 0.0f32;
    let mut buf = [0u8; 4];
    for (seg, btype) in &segments {
        let seg_w = measure_width(seg);
        let seg_chars = seg.chars().count();

        // 超长词边界（§5）：segment 本身超 max_w 且多字符 → 逐字填
        // （参考 fgui BuildLines toMoveChars=1；防无 break point 的长串如 URL 溢出）。
        if !nowrap && seg_w > max_w && seg_chars > 1 {
            if !cur.is_empty() {
                lines.push((std::mem::take(&mut cur), cur_w));
                cur_w = 0.0;
            }
            for ch in seg.chars() {
                let cw = measure_width(ch.encode_utf8(&mut buf));
                if !cur.is_empty() && cur_w + cw > max_w {
                    lines.push((std::mem::take(&mut cur), cur_w));
                    cur_w = 0.0;
                }
                cur.push(ch);
                cur_w += cw;
            }
        } else if nowrap || cur.is_empty() || cur_w + seg_w <= max_w {
            cur.push_str(seg);
            cur_w += seg_w;
        } else {
            lines.push((std::mem::take(&mut cur), cur_w));
            cur.push_str(seg);
            cur_w = seg_w;
        }

        // Mandatory break（\n）强制结束当前行（nowrap 下忽略）。
        if !nowrap && *btype == BreakOpportunity::Mandatory && !cur.is_empty() {
            lines.push((std::mem::take(&mut cur), cur_w));
            cur_w = 0.0;
        }
    }
    if !cur.is_empty() {
        lines.push((cur, cur_w));
    }
    if lines.is_empty() {
        lines.push((String::new(), 0.0));
    }

    let text_width = lines
        .iter()
        .map(|(_, w)| *w)
        .fold(0.0f32, f32::max);
    let text_height = lines.len() as f32 * line_h;

    // 生成 glyphs（绝对坐标，§9.2：已累加 advance + 已应用 align 偏移）。
    let mut out_lines = Vec::with_capacity(lines.len());
    for (li, (text, lw)) in lines.iter().enumerate() {
        let line_y = li as f32 * line_h;
        let x_offset = match align {
            crate::style::resolved::TextAlign::Center => (text_width - lw) / 2.0,
            crate::style::resolved::TextAlign::Right => text_width - lw,
            crate::style::resolved::TextAlign::Left => 0.0,
        };
        let mut pen_x = x_offset;
        let mut glyphs = Vec::with_capacity(text.chars().count());
        let mut prev: Option<ttf_parser::GlyphId> = None;
        for ch in text.chars() {
            let gid = font.face.glyph_index(ch).unwrap_or_default();
            if let Some(p) = prev {
                if let Some(k) = kerning(p, gid) {
                    pen_x += to_px(k as f32);
                }
            }
            // bearing 来自 glyph bbox：x_min → bearing_x，y_max → bearing_y（顶到 baseline）。
            let (bx, by) = font
                .face
                .glyph_bounding_box(gid)
                .map(|b| (to_px(b.x_min as f32), to_px(b.y_max as f32)))
                .unwrap_or((0.0, 0.0));
            glyphs.push(Glyph {
                glyph_id: gid.0,
                codepoint: ch as u32,
                x: pen_x,
                y: line_y,
                bearing_x: bx,
                bearing_y: by,
            });
            pen_x += advance(gid) + letter_spacing;
            prev = Some(gid);
        }
        out_lines.push(Line {
            y: line_y,
            height: line_h,
            baseline: line_y + baseline,
            width: *lw,
            runs: vec![GlyphRun {
                font_size,
                glyphs,
            }],
        });
    }

    TextLayout {
        text_width,
        text_height,
        lines: out_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::resolved::TextAlign;

    /// 测试字体：仓库内 DejaVuSans.ttf（跨平台一致），缺则跳过。
    fn test_font() -> Option<Font> {
        let p = format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"));
        Font::from_path(&p).ok()
    }

    /// CJK 测试字体：仓库内 wqy-microhei.ttc（文泉驿微米黑），缺则跳过。
    /// .ttc 用 Face::parse(bytes, 0) 取 index 0 face。
    fn test_font_cjk() -> Option<Font> {
        let p = format!("{}/tests/fixtures/wqy-microhei.ttc", env!("CARGO_MANIFEST_DIR"));
        Font::from_path(&p).ok()
    }

    #[test]
    fn cjk_font_loads_and_has_cjk_glyph_advance() {
        let font = match test_font_cjk() {
            Some(f) => f,
            None => {
                eprintln!("skip: no CJK test font (wqy-microhei.ttc)");
                return;
            }
        };
        // CJK 字符「中」应有 glyph（非 .notdef=0）且 advance > 0。
        let gid = font.face.glyph_index('中');
        assert!(gid.is_some(), "CJK 字体应含「中」glyph");
        let adv = font.face.glyph_hor_advance(gid.unwrap());
        assert!(adv.is_some() && adv.unwrap() > 0, "「中」advance 应 > 0");
        // 度量方法可用。
        assert!(font.ascent(16.0) > 0.0);
    }

    #[test]
    fn single_line_ascii_has_glyphs() {
        let font = match test_font() {
            Some(f) => f,
            None => {
                eprintln!("skip: no test font");
                return;
            }
        };
        let layout = measure_text("Hello", 16.0, 0.0, 0.0, TextAlign::Left, false, None, &font);
        assert_eq!(layout.lines.len(), 1);
        assert!(!layout.lines[0].runs.is_empty());
        // Hello = 5 字形
        assert_eq!(layout.lines[0].runs[0].glyphs.len(), 5);
        assert!(layout.text_width > 0.0);
    }

    #[test]
    fn wraps_on_width_constraint() {
        let font = match test_font() {
            Some(f) => f,
            None => {
                eprintln!("skip: no test font");
                return;
            }
        };
        let layout = measure_text(
            "aaaa bbbb cccc",
            16.0,
            0.0,
            0.0,
            TextAlign::Left,
            false,
            Some(50.0),
            &font,
        );
        assert!(
            layout.lines.len() >= 2,
            "应在窄约束下换行，得 {} 行",
            layout.lines.len()
        );
    }

    #[test]
    fn nowrap_never_wraps() {
        let font = match test_font() {
            Some(f) => f,
            None => {
                eprintln!("skip: no test font");
                return;
            }
        };
        let layout = measure_text(
            "aaaa bbbb cccc",
            16.0,
            0.0,
            0.0,
            TextAlign::Left,
            true,
            Some(10.0),
            &font,
        );
        assert_eq!(layout.lines.len(), 1);
    }

    #[test]
    fn cjk_breaks_per_char_under_narrow_width() {
        let font = match test_font_cjk() {
            Some(f) => f,
            None => { eprintln!("skip: no CJK test font"); return; }
        };
        // 8 个 CJK 字符，窄约束（每字 ~font_size 宽）→ 应逐字断 ≥2 行。
        let layout = measure_text(
            "你好世界字体测试", 16.0, 0.0, 0.0, TextAlign::Left, false, Some(40.0), &font,
        );
        assert!(
            layout.lines.len() >= 2,
            "CJK 窄约束应逐字换行，得 {} 行",
            layout.lines.len()
        );
    }

    #[test]
    fn cjk_ascii_mix_breaks_correctly() {
        let font = match test_font_cjk() {
            Some(f) => f,
            None => { eprintln!("skip: no CJK test font"); return; }
        };
        // CJK + ASCII 混排，窄约束 → 多行；不 panic、不出空行。
        let layout = measure_text(
            "Hello 世界 ABC 测试", 16.0, 0.0, 0.0, TextAlign::Left, false, Some(60.0), &font,
        );
        assert!(layout.lines.len() >= 2, "混排窄约束应换行");
        // 每行至少有 glyph（无空行）。
        for line in &layout.lines {
            let glyph_count: usize = line.runs.iter().map(|r| r.glyphs.len()).sum();
            assert!(glyph_count > 0, "不应有空行");
        }
    }

    #[test]
    fn newline_is_mandatory_break() {
        let font = match test_font() { Some(f) => f, None => { eprintln!("skip"); return; } };
        // \n 应强制换行（v0 split(' ') 不处理 \n，本 task 改进）。
        let layout = measure_text(
            "aaaa\nbbbb", 16.0, 0.0, 0.0, TextAlign::Left, false, None, &font,
        );
        assert_eq!(layout.lines.len(), 2, "\\n 应强制换行成 2 行");
    }

    #[test]
    fn nowrap_keeps_cjk_single_line() {
        let font = match test_font_cjk() {
            Some(f) => f,
            None => { eprintln!("skip: no CJK test font"); return; }
        };
        let layout = measure_text(
            "你好世界字体测试", 16.0, 0.0, 0.0, TextAlign::Left, true, Some(10.0), &font,
        );
        assert_eq!(layout.lines.len(), 1, "nowrap 强制单行（含 CJK）");
    }

    #[test]
    fn super_long_word_breaks_per_char() {
        let font = match test_font() { Some(f) => f, None => { eprintln!("skip"); return; } };
        // 无空格长 ASCII 串（超 max_w）→ 超长词边界：逐字断（§5，参考 fgui toMoveChars=1）。
        let layout = measure_text(
            "aaaaaaaaaaaaaaaaaaaa", 16.0, 0.0, 0.0, TextAlign::Left, false, Some(50.0), &font,
        );
        assert!(layout.lines.len() >= 2, "超长无空格串应逐字断 ≥2 行");
    }

    #[test]
    fn line_height_scales_line_box() {
        let font = match test_font() {
            Some(f) => f,
            None => {
                eprintln!("skip: no test font");
                return;
            }
        };
        let normal = measure_text("Hi", 16.0, 0.0, 0.0, TextAlign::Left, false, None, &font);
        let tall = measure_text("Hi", 16.0, 2.0, 0.0, TextAlign::Left, false, None, &font);
        assert!(tall.lines[0].height > normal.lines[0].height);
    }
}
