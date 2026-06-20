//! 纹理注册表（v1b.3 atlas）：src → TexMeta{tex_id, uv_min, uv_max, w, h}。per Stage。
//! atlas 模型：core 在 load_package 时从 AtlasSprite 表建（asset::build_registry），
//! 同图集 sprite 共享 tex_id。tex_id 0 保留 = 未注册哨兵。
//! core 只持整数 id + UV region + 维度；GPU 纹理由后端持有。

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexMeta {
    pub tex_id: u32,        // atlas root tex_id（同图集 sprite 共享）；0=未注册哨兵
    pub uv_min: [f32; 2],   // sprite 在 atlas 内 UV 左上（核心 y-down 约定，[0,1]）
    pub uv_max: [f32; 2],   // UV 右下
    pub width: u32,         // sprite 原始像素宽（measure；甲-B = region.w，无 trim）
    pub height: u32,        // sprite 原始像素高
}

#[derive(Debug, Default)]
pub struct TextureRegistry {
    map: HashMap<String, TexMeta>,
}

impl TextureRegistry {
    pub fn get(&self, src: &str) -> Option<TexMeta> { self.map.get(src).copied() }
    /// load_package 从 AtlasSprite 表建时插入（build_registry），或测试手工插。
    pub fn insert(&mut self, src: &str, meta: TexMeta) { self.map.insert(src.into(), meta); }
    pub fn clear(&mut self) { self.map.clear(); }
    pub fn len(&self) -> usize { self.map.len() }
    pub fn is_empty(&self) -> bool { self.map.is_empty() }

    /// v1b.2 loose-collect FFI 注册（`loomgui_stage_register_texture`）：分配单调
    /// tex_id（从 1 起；0=未注册哨兵），src 幂等返回同 id。
    ///
    /// v1b.3 atlas 模型下，包路径已由 `build_registry` 一次性建表（uv_min/uv_max
    /// 来自 AtlasSprite）；此 `register` 仅供仍存在的 loose-collect FFI（Unity 旧
    /// collect→register 握手）使用，T5 删该 FFI 时一并删本方法。维度首写入胜，
    /// UV 默认整图 [0,0]-[1,1]（loose 图非 atlas 子区）。
    pub fn register(&mut self, src: &str, width: u32, height: u32) -> u32 {
        if let Some(m) = self.map.get(src) {
            return m.tex_id;
        }
        // tex_id = 当前最大 + 1（map 空时 1）；与 v1b.2 next_id 语义等价。
        let id = self.map.values().map(|m| m.tex_id).max().unwrap_or(0) + 1;
        self.map.insert(src.into(), TexMeta {
            tex_id: id,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            width,
            height,
        });
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(tid: u32, w: u32, h: u32) -> TexMeta {
        TexMeta { tex_id: tid, uv_min: [0.0, 0.0], uv_max: [1.0, 1.0], width: w, height: h }
    }

    #[test]
    fn insert_and_get_round_trips() {
        let mut r = TextureRegistry::default();
        r.insert("a.png", meta(1, 10, 20));
        let m = r.get("a.png").unwrap();
        assert_eq!((m.tex_id, m.width, m.height), (1, 10, 20));
        assert_eq!(m.uv_min, [0.0, 0.0]);
        assert_eq!(m.uv_max, [1.0, 1.0]);
    }

    #[test]
    fn get_miss_returns_none() {
        let r = TextureRegistry::default();
        assert!(r.get("nope.png").is_none());
    }

    #[test]
    fn insert_overwrites_same_src() {
        let mut r = TextureRegistry::default();
        r.insert("a.png", meta(1, 10, 20));
        r.insert("a.png", meta(2, 30, 40));   // 覆盖
        let m = r.get("a.png").unwrap();
        assert_eq!((m.tex_id, m.width), (2, 30));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn clear_empties_registry() {
        let mut r = TextureRegistry::default();
        r.insert("a.png", meta(1, 1, 1));
        r.clear();
        assert!(r.is_empty());
        assert!(r.get("a.png").is_none());
    }
}
