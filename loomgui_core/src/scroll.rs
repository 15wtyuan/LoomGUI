//! v1d.5 ScrollPane 状态 + 物理（spec §2/§4）。transient（不进 pkg）。
//!
//! 本模块（T3）只持数据模型：
//! - `ScrollPaneState`：每滚动容器几何（content/viewport/overlap）+ 物理状态（pos/velocity/tween）。
//!   物理状态字段本任务定义但 advance 逻辑不实现（T6）；`#[derive(Default)]` 全 0/false。
//! - `ScrollTable`：per-node 槽（`Vec<Option<ScrollPaneState>>`，NodeId 索引），镜像 `AnimTable` 模式。
//! - `refresh_content_sizes(&mut Scene)`：layout solve 后填 content_size/viewport/overlap。
//! - `capable` / `effective` helper。
//!
//! core 无 Vec2 类型——几何用 `(f32, f32)` 元组（照 `transform::apply_point`）。

use crate::scene::node::{NodeId, Node, Scene};
use crate::style::resolved::OverflowMode;

/// 单滚动容器状态。`#[derive(Default)]`：几何全 0、物理全 0/false、tweening=0（无）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollPaneState {
    /// 直接子 layout_rect 的 AABB 尺寸（spec §2.3）。
    pub content_size: (f32, f32),
    /// 本容器 content box 尺寸（v1 = layout_rect border box；spec §1.3 认 padding 简化）。
    pub viewport_size: (f32, f32),
    /// (content - viewport).max(0) 每轴；负钳 0。
    pub overlap: (f32, f32),
    /// 当前滚动位置（content 坐标系偏移）。
    pub scroll_pos: (f32, f32),
    /// 惯性速度（px/s）。advance 写（T6）。
    pub velocity: (f32, f32),
    /// 0=无补间，1=set_pos 补间，2=惯性+回弹补间。advance 写（T6）。
    pub tweening: u8,
    pub tween_start: (f32, f32),
    pub tween_change: (f32, f32),
    pub tween_time: (f32, f32),
    pub tween_duration: (f32, f32),
    /// refresh 后若 content_size 变化置 true（供 scrollbar 复布局用，T9）。
    pub content_size_dirty: bool,
}

/// 每节点滚动状态表（`Vec<Option<ScrollPaneState>>`，index = NodeId.0）。
/// 镜像 `AnimTable` 模式但槽为 `Option`（仅滚动容器 ensure 后有值）。
/// transient——不进 pkg（同 `anim` / `world_transforms`）。
#[derive(Debug, Clone, Default)]
pub struct ScrollTable(pub Vec<Option<ScrollPaneState>>);

impl ScrollTable {
    pub fn get(&self, id: NodeId) -> Option<&ScrollPaneState> {
        self.0.get(id.0).and_then(|o| o.as_ref())
    }
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut ScrollPaneState> {
        self.0.get_mut(id.0).and_then(|o| o.as_mut())
    }
    /// 增长到含 id 的长度（缺省填 None），返回该节点可变状态（缺则插 default）。
    pub fn ensure(&mut self, id: NodeId) -> &mut ScrollPaneState {
        if id.0 >= self.0.len() {
            self.0.resize(id.0 + 1, None);
        }
        self.0[id.0].get_or_insert_with(ScrollPaneState::default)
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// 该轴是否允许滚动（overflow ∈ {Scroll, Auto}）。
pub fn capable(ovf: OverflowMode) -> bool {
    matches!(ovf, OverflowMode::Scroll | OverflowMode::Auto)
}

/// 该轴实际可滚（capable 且 (Scroll 或 content > viewport)）。
/// Auto 仅当内容溢出才可滚；Scroll 无论溢出与否皆可滚（fgui 语义）。
pub fn effective(ovf: OverflowMode, content: f32, viewport: f32) -> bool {
    capable(ovf) && (ovf == OverflowMode::Scroll || content > viewport)
}

/// solve 后填 content_size/viewport/overlap（spec §2.3）。
/// 遍历节点：任一轴 overflow != Visible 即视为滚动容器，ensure 后写几何。
/// children clone 避借用冲突（遍历子 layout_rect 时也要借 scene.nodes）。
pub fn refresh_content_sizes(scene: &mut Scene) {
    let ids: Vec<usize> = (0..scene.nodes.len()).collect();
    for id in ids {
        let nid = NodeId(id);
        let is_scroll = {
            let n = &scene.nodes[id];
            n.style.overflow_x != OverflowMode::Visible
                || n.style.overflow_y != OverflowMode::Visible
        };
        if !is_scroll {
            continue;
        }
        // content_size = 直接子节点 layout_rect AABB。
        let kids = scene.nodes[id].children.clone();
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for c in &kids {
            let r = scene.nodes[c.0].layout_rect;
            min_x = min_x.min(r.x);
            min_y = min_y.min(r.y);
            max_x = max_x.max(r.x + r.w);
            max_y = max_y.max(r.y + r.h);
        }
        let content = if kids.is_empty() {
            (0.0, 0.0)
        } else {
            ((max_x - min_x).max(0.0), (max_y - min_y).max(0.0))
        };
        let viewport = content_box_size(&scene.nodes[id]);
        let st = scene.scroll.ensure(nid);
        st.content_size_dirty = st.content_size != content;
        st.content_size = content;
        st.viewport_size = viewport;
        st.overlap = (
            (content.0 - viewport.0).max(0.0),
            (content.1 - viewport.1).max(0.0),
        );
    }
}

/// content box 尺寸。v1 简化：用 border box（layout_rect 尺寸）。
/// spec §1.3 已声明 padding 简化（建议 scroll 容器 padding:0）；padding 边缘处理 defer。
fn content_box_size(node: &Node) -> (f32, f32) {
    let lr = node.layout_rect;
    (lr.w, lr.h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::{NodeKind, Rect};
    use crate::style::resolved::ResolvedStyle;

    /// 构造滚动测试场景：
    ///   node 0 = scroll 容器（overflow_y=Scroll），layout_rect (0,0,100,100)
    ///   node 1 = 子，layout_rect (0,0,40,40)
    ///   node 2 = 子，layout_rect (0,50,30,30)
    ///   node 3 = 非 scroll（overflow 双轴 Visible）
    /// content AABB = (max_right 40, max_bottom 80)。
    fn build_scroll_scene() -> Scene {
        let mut scroll_style = ResolvedStyle::default();
        scroll_style.overflow_y = OverflowMode::Scroll;
        let entries: Vec<(
            Option<usize>,
            NodeKind,
            ResolvedStyle,
            Vec<String>,
            Option<String>,
            bool,
            Option<i32>,
        )> = vec![
            (None, NodeKind::Container, scroll_style.clone(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (Some(0), NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
            (None, NodeKind::Container, ResolvedStyle::default(), vec![], None, false, None),
        ];
        let mut s = Scene::build(&entries);
        s.nodes[0].layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        s.nodes[1].layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.nodes[2].layout_rect = Rect { x: 0.0, y: 50.0, w: 30.0, h: 30.0 };
        s.nodes[3].layout_rect = Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 };
        s
    }

    #[test]
    fn content_size_is_children_aabb() {
        let mut s = build_scroll_scene();
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(NodeId(0)).expect("scroll 容器有 state");
        assert!(
            (st.content_size.0 - 40.0).abs() < 1e-3 && (st.content_size.1 - 80.0).abs() < 1e-3,
            "content_size = (40, 80)，got {:?}",
            st.content_size
        );
    }

    #[test]
    fn viewport_and_overlap_from_geometry() {
        let mut s = build_scroll_scene();
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(NodeId(0)).unwrap();
        // viewport = layout_rect border box = (100, 100)
        assert!((st.viewport_size.0 - 100.0).abs() < 1e-3);
        assert!((st.viewport_size.1 - 100.0).abs() < 1e-3);
        // overlap = max(content - viewport, 0) = (0, 0) 因 content < viewport 各轴
        // 注：content=(40,80) < viewport=(100,100) → overlap (0,0)
        assert_eq!(st.overlap, (0.0, 0.0));
    }

    #[test]
    fn overlap_clamps_negative_to_zero() {
        // content < viewport → overlap 0（与上一测同场景，显式命名）
        let mut s = build_scroll_scene();
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(NodeId(0)).unwrap();
        assert_eq!(st.overlap, (0.0, 0.0));
    }

    #[test]
    fn overlap_positive_when_content_exceeds_viewport() {
        // 改子 layout_rect 让 content > viewport y 轴
        let mut s = build_scroll_scene();
        s.nodes[1].layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.nodes[2].layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 };
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(NodeId(0)).unwrap();
        // content = (40, 200)；viewport = (100,100) → overlap = (0, 100)
        assert!(
            (st.overlap.0 - 0.0).abs() < 1e-3 && (st.overlap.1 - 100.0).abs() < 1e-3,
            "overlap y = 100，got {:?}",
            st.overlap
        );
    }

    #[test]
    fn non_scroll_node_has_no_state() {
        let mut s = build_scroll_scene();
        refresh_content_sizes(&mut s);
        // node 3 双轴 Visible → 非 scroll 容器 → scroll.get 返 None
        assert!(s.scroll.get(NodeId(3)).is_none(), "非 scroll 节点无 state");
    }

    #[test]
    fn capable_and_effective_semantics() {
        // capable: Scroll/Auto true；Visible/Hidden false
        assert!(capable(OverflowMode::Scroll));
        assert!(capable(OverflowMode::Auto));
        assert!(!capable(OverflowMode::Visible));
        assert!(!capable(OverflowMode::Hidden));
        // effective: Scroll 永真（capable 且 == Scroll）；Auto 仅 content>viewport
        assert!(effective(OverflowMode::Scroll, 10.0, 100.0), "Scroll 即使 content<viewport 仍可滚");
        assert!(effective(OverflowMode::Auto, 200.0, 100.0), "Auto content>viewport 可滚");
        assert!(!effective(OverflowMode::Auto, 50.0, 100.0), "Auto content<viewport 不可滚");
        assert!(!effective(OverflowMode::Visible, 200.0, 100.0), "Visible 不可滚");
    }

    #[test]
    fn scrolltable_get_mut_ensure_clear() {
        let mut t = ScrollTable::default();
        assert!(t.get(NodeId(0)).is_none(), "空表 get → None");
        // ensure 增长并插 default
        let st = t.ensure(NodeId(2));
        st.scroll_pos = (5.0, 7.0);
        assert_eq!(t.0.len(), 3, "ensure(2) 增长到 len 3");
        let got = t.get(NodeId(2)).unwrap();
        assert_eq!(got.scroll_pos, (5.0, 7.0));
        // get_mut
        {
            let m = t.get_mut(NodeId(2)).unwrap();
            m.scroll_pos = (1.0, 2.0);
        }
        assert_eq!(t.get(NodeId(2)).unwrap().scroll_pos, (1.0, 2.0));
        // ensure 同 id 二次返同槽（不重置）
        let st2 = t.ensure(NodeId(2));
        assert_eq!(st2.scroll_pos, (1.0, 2.0), "二次 ensure 不重置已有值");
        // clear
        t.clear();
        assert!(t.0.is_empty(), "clear 清空");
    }

    #[test]
    fn content_size_dirty_flag_when_changes() {
        let mut s = build_scroll_scene();
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(NodeId(0)).unwrap();
        // 首次：原 default (0,0) → (40,80) → dirty true
        assert!(st.content_size_dirty, "首次填入非零 content → dirty");
        // 再 refresh 一次（content 不变）→ dirty false
        refresh_content_sizes(&mut s);
        let st2 = s.scroll.get(NodeId(0)).unwrap();
        assert!(!st2.content_size_dirty, "content 未变 → dirty false");
        // 改子尺寸 → dirty true
        s.nodes[2].layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 };
        refresh_content_sizes(&mut s);
        let st3 = s.scroll.get(NodeId(0)).unwrap();
        assert!(st3.content_size_dirty, "content 变 → dirty true");
    }

    #[test]
    fn empty_children_content_is_zero() {
        // 滚动容器无子 → content (0,0)
        let mut style = ResolvedStyle::default();
        style.overflow_y = OverflowMode::Scroll;
        let entries = vec![
            (None, NodeKind::Container, style, vec![], None, false, None),
        ];
        let mut s = Scene::build(&entries);
        s.nodes[0].layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(NodeId(0)).unwrap();
        assert_eq!(st.content_size, (0.0, 0.0), "无子 content = (0,0)");
        assert_eq!(st.overlap, (0.0, 0.0));
    }
}
