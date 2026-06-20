//! 纹理注册表（spec §5.1）：src → TexMeta{tex_id, w, h}。per Stage。
//! core 只持整数 id + 维度；GPU 纹理由后端持有。tex_id 0 保留 = 未注册哨兵。

use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct TexMeta {
    pub tex_id: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub struct TextureRegistry {
    map: HashMap<String, TexMeta>,
    next_id: u32, // 从 1 起；0 保留 = 未注册哨兵
}

impl Default for TextureRegistry {
    fn default() -> Self {
        Self { map: HashMap::new(), next_id: 1 }
    }
}

impl TextureRegistry {
    /// 注册：core 分配 tex_id（src 幂等，同 src 二次调用返回同 id、维度首次胜）。
    /// 返回 tex_id(>=1)。
    pub fn register(&mut self, src: &str, w: u32, h: u32) -> u32 {
        if let Some(m) = self.map.get(src) {
            return m.tex_id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.map.insert(src.to_string(), TexMeta { tex_id: id, width: w, height: h });
        id
    }

    pub fn get(&self, src: &str) -> Option<TexMeta> {
        self.map.get(src).copied()
    }

    /// 重载用：清表 + next_id 重置回 1。
    pub fn clear(&mut self) {
        self.map.clear();
        self.next_id = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_assigns_monotonic_ids_starting_at_1() {
        let mut r = TextureRegistry::default();
        assert_eq!(r.register("a.png", 10, 20), 1);
        assert_eq!(r.register("b.png", 30, 40), 2);
        assert_eq!(r.register("c.png", 50, 60), 3);
    }

    #[test]
    fn register_is_idempotent_on_src_first_dims_win() {
        let mut r = TextureRegistry::default();
        assert_eq!(r.register("a.png", 10, 20), 1);
        // 同 src 二次：返回同 id，next_id 不前进，维度不被覆盖。
        assert_eq!(r.register("a.png", 999, 999), 1);
        assert_eq!(r.register("b.png", 1, 1), 2);
        let m = r.get("a.png").unwrap();
        assert_eq!((m.tex_id, m.width, m.height), (1, 10, 20));
    }

    #[test]
    fn get_miss_returns_none() {
        let r = TextureRegistry::default();
        assert!(r.get("nope.png").is_none());
    }

    #[test]
    fn clear_resets_ids_and_map() {
        let mut r = TextureRegistry::default();
        r.register("a.png", 1, 1);
        r.register("b.png", 2, 2);
        r.clear();
        assert!(r.get("a.png").is_none());
        // clear 后再注册从 1 起。
        assert_eq!(r.register("c.png", 3, 3), 1);
    }

    #[test]
    fn zero_is_never_assigned() {
        let mut r = TextureRegistry::default();
        for i in 0..10 {
            assert_ne!(r.register(&format!("f{i}.png"), 1, 1), 0);
        }
    }
}
