//! GTween-lite：tween 引擎（TweenManager + Ease/TweenProp）。
//! 动 opacity / transform(translate·scale·rotate) / 颜色(bg·text)。
//! replace-override：动画值覆盖 ResolvedStyle 读取点（None 退回 CSS）。

/// 可动属性。u8 值与 FFI / C# enum 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TweenProp {
    Opacity = 0,
    Translate = 1,
    Scale = 2,
    Rotation = 3,
    BgColor = 4,
    TextColor = 5,
}

impl TweenProp {
    /// u32 → TweenProp（FFI 校验用）。越界 → None。判别值与 C# enum / FFI u32 对齐。
    pub fn try_from(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Opacity),
            1 => Some(Self::Translate),
            2 => Some(Self::Scale),
            3 => Some(Self::Rotation),
            4 => Some(Self::BgColor),
            5 => Some(Self::TextColor),
            _ => None,
        }
    }
}

/// 每个 prop 的 lerp 分量数（start/end [f32;4] 取前 N 个）。
pub fn prop_value_size(prop: TweenProp) -> u8 {
    match prop {
        TweenProp::Opacity | TweenProp::Rotation => 1,
        TweenProp::Translate | TweenProp::Scale => 2,
        TweenProp::BgColor | TweenProp::TextColor => 4,
    }
}

/// easing 子集（10 个）。u8 值与 FFI / C# enum 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Ease {
    Linear = 0,
    QuadIn = 1,
    QuadOut = 2,
    QuadInOut = 3,
    CubicIn = 4,
    CubicOut = 5,
    CubicInOut = 6,
    BackIn = 7,
    BackOut = 8,
    BackInOut = 9,
}

impl Ease {
    /// u32 → Ease（FFI 校验用）。越界 → None。判别值与 C# enum / FFI u32 对齐。
    pub fn try_from(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Linear),
            1 => Some(Self::QuadIn),
            2 => Some(Self::QuadOut),
            3 => Some(Self::QuadInOut),
            4 => Some(Self::CubicIn),
            5 => Some(Self::CubicOut),
            6 => Some(Self::CubicInOut),
            7 => Some(Self::BackIn),
            8 => Some(Self::BackOut),
            9 => Some(Self::BackInOut),
            _ => None,
        }
    }
}

const OVERSHOOT: f32 = 1.70158;

impl Ease {
    /// t∈[0,dur] → [0,1]。dur<=0 直接返 1（调用方已钳 tt<=dur，这里防除零）。
    pub fn evaluate(self, t: f32, dur: f32) -> f32 {
        if dur <= 0.0 {
            return 1.0;
        }
        match self {
            Ease::Linear => t / dur,
            Ease::QuadIn => { let t = t / dur; t * t }
            Ease::QuadOut => { let t = t / dur; -t * (t - 2.0) }
            Ease::QuadInOut => {
                let t = t / (dur * 0.5);
                if t < 1.0 { 0.5 * t * t }
                else { let t = t - 1.0; -0.5 * (t * (t - 2.0) - 1.0) }
            }
            Ease::CubicIn => { let t = t / dur; t * t * t }
            Ease::CubicOut => { let t = t / dur - 1.0; t * t * t + 1.0 }
            Ease::CubicInOut => {
                let t = t / (dur * 0.5);
                if t < 1.0 { 0.5 * t * t * t }
                else { let t = t - 2.0; 0.5 * (t * t * t + 2.0) }
            }
            Ease::BackIn => { let t = t / dur; t * t * ((OVERSHOOT + 1.0) * t - OVERSHOOT) }
            Ease::BackOut => { let t = t / dur - 1.0; t * t * ((OVERSHOOT + 1.0) * t + OVERSHOOT) + 1.0 }
            Ease::BackInOut => {
                let s = OVERSHOOT * 1.525;
                let t = t / (dur * 0.5);
                if t < 1.0 { 0.5 * (t * t * ((s + 1.0) * t - s)) }
                else { let t = t - 2.0; 0.5 * (t * t * ((s + 1.0) * t + s) + 2.0) }
            }
        }
    }
}

use crate::input::{EventRecord, EVT_TWEEN_COMPLETE};
use crate::scene::node::{NodeId, NodeAnim, Scene};
use crate::transform::{self};

/// 一个进行中的 tween（内部结构，TweenManager 内部 Vec 管理）。
#[derive(Debug, Clone)]
struct Tween {
    node: NodeId,
    prop: TweenProp,
    start: [f32; 4],
    end: [f32; 4],
    ease: Ease,
    delay: f32,
    duration: f32,
    elapsed: f32,
    tag: u32,
    started: bool,
    killed: bool,
}

/// Tween 引擎：持一组 Tween，每 tick 推进、写 scene.anim，完成时产 EVT_TWEEN_COMPLETE。
#[derive(Debug, Default)]
pub struct TweenManager {
    tweens: Vec<Tween>,
}

impl TweenManager {
    pub fn new() -> Self { Self { tweens: Vec::new() } }

    /// 清空所有 tween（load 重建 scene 时调，防残留指向失效 node_id）。
    pub fn clear(&mut self) { self.tweens.clear(); }

    /// 注册一个 tween。越界 node 由 update 跳过。
    pub fn tween(
        &mut self, node: NodeId, prop: TweenProp,
        start: [f32; 4], end: [f32; 4],
        ease: Ease, delay: f32, duration: f32, tag: u32,
    ) {
        self.tweens.push(Tween {
            node, prop, start, end,
            ease, delay, duration,
            elapsed: 0.0, tag, started: false, killed: false,
        });
    }

    /// 停该节点该 prop 的 tween（killed，override 保留末值）。
    pub fn kill(&mut self, node: NodeId, prop: TweenProp) {
        for t in &mut self.tweens {
            if t.node == node && t.prop == prop && !t.killed {
                t.killed = true;
            }
        }
    }

    /// 每 tick：推进 active tween，写 scene.anim，产 complete 事件。
    pub fn update(&mut self, dt: f32, scene: &mut Scene, out: &mut Vec<EventRecord>) {
        if self.tweens.is_empty() {
            return;
        }
        let n = scene.nodes.len();
        let anim = scene.anim.ensure(n);
        // 先在 anim 上 apply（按 node 索引），再单独收集 complete 事件。
        for t in &mut self.tweens {
            if t.killed || t.node.0 >= n {
                continue;
            }
            t.elapsed += dt;
            if t.elapsed < t.delay {
                continue;
            }
            t.started = true;
            let tt = t.elapsed - t.delay;
            let clamped = if tt >= t.duration { t.duration } else { tt };
            let norm = t.ease.evaluate(clamped, t.duration);
            apply(anim, t.node, t.prop, t.start, t.end, norm);
            if tt >= t.duration {
                t.killed = true;
                out.push(EventRecord {
                    node_id: t.node.0 as u32,
                    event_type: EVT_TWEEN_COMPLETE,
                    click_count: t.prop as u8,       // 复用：prop 枚举值
                    pad: [0, 0],
                    touch_id: t.tag as i32,          // 复用：调用方 tag
                    x: 0.0, y: 0.0,
                });
            }
        }
        self.tweens.retain(|t| !t.killed);
    }
}

/// 逐分量 lerp start→end 写入 anim 对应通道（n=已算的 normalized）。
fn apply(anim: &mut [NodeAnim], node: NodeId, prop: TweenProp, start: [f32; 4], end: [f32; 4], n: f32) {
    let a = &mut anim[node.0];
    let lerp = |i: usize| start[i] + (end[i] - start[i]) * n;
    match prop {
        TweenProp::Opacity => a.opacity = Some(lerp(0)),
        TweenProp::Translate => a.transform = Some(transform::from_translate(lerp(0), lerp(1))),
        TweenProp::Scale => a.transform = Some(transform::from_scale(lerp(0), lerp(1))),
        TweenProp::Rotation => a.transform = Some(transform::from_rotate(lerp(0))),
        TweenProp::BgColor => a.bg_color = Some([lerp(0), lerp(1), lerp(2), lerp(3)]),
        TweenProp::TextColor => a.text_color = Some([lerp(0), lerp(1), lerp(2), lerp(3)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prop_value_size_mapping() {
        assert_eq!(prop_value_size(TweenProp::Opacity), 1);
        assert_eq!(prop_value_size(TweenProp::Rotation), 1);
        assert_eq!(prop_value_size(TweenProp::Translate), 2);
        assert_eq!(prop_value_size(TweenProp::Scale), 2);
        assert_eq!(prop_value_size(TweenProp::BgColor), 4);
        assert_eq!(prop_value_size(TweenProp::TextColor), 4);
    }

    #[test]
    fn ease_endpoints_are_0_and_1() {
        let dur = 1.0;
        for ease in [
            Ease::Linear, Ease::QuadIn, Ease::QuadOut, Ease::QuadInOut,
            Ease::CubicIn, Ease::CubicOut, Ease::CubicInOut,
            Ease::BackIn, Ease::BackOut, Ease::BackInOut,
        ] {
            assert!((ease.evaluate(0.0, dur)).abs() < 1e-5, "{:?}@0 != 0", ease);
            assert!((ease.evaluate(dur, dur) - 1.0).abs() < 1e-5, "{:?}@dur != 1", ease);
        }
    }

    #[test]
    fn ease_dur_zero_returns_1() {
        assert_eq!(Ease::Linear.evaluate(0.5, 0.0), 1.0);
    }

    #[test]
    fn cubic_in_below_linear_below_cubic_out_at_mid() {
        // t=0.5,dur=1：CubicIn(0.125) < Linear(0.5) < CubicOut(0.875)
        let lin = Ease::Linear.evaluate(0.5, 1.0);
        let cin = Ease::CubicIn.evaluate(0.5, 1.0);
        let cout = Ease::CubicOut.evaluate(0.5, 1.0);
        assert!(cin < lin && lin < cout, "CubicIn({}) < Linear({}) < CubicOut({})", cin, lin, cout);
        assert!((cin - 0.125).abs() < 1e-5);
        assert!((cout - 0.875).abs() < 1e-5);
    }

    #[test]
    fn back_out_overshoots_above_1_mid() {
        // BackOut 中段 >1（overshoot）；约 t≈0.6 处达峰 ~1.1
        let mut max_v = 0.0f32;
        for i in 0..100 {
            let t = i as f32 / 100.0;
            let v = Ease::BackOut.evaluate(t, 1.0);
            if v > max_v { max_v = v; }
        }
        assert!(max_v > 1.0, "BackOut 中段须 >1（overshoot），实达 {}", max_v);
    }

    #[test]
    fn back_in_undershoots_below_0_early() {
        // BackIn 初段 <0（反向 overshoot）
        let v = Ease::BackIn.evaluate(0.1, 1.0);
        assert!(v < 0.0, "BackIn 初段须 <0，得 {}", v);
    }

    // ===== TweenManager 测 =====

    use crate::scene::node::{AnimTable, Node, NodeKind, Rect};
    use crate::input::EVT_TWEEN_COMPLETE;

    fn one_node_scene() -> Scene {
        let mut n = Node::default();
        n.id = NodeId(0);
        n.kind = NodeKind::Container;
        n.layout_rect = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        Scene {
            roots: vec![NodeId(0)],
            nodes: vec![n],
            dynamic_rules: Default::default(),
            focused_node: None,
            world_transforms: Vec::new(),
            anim: AnimTable::default(),
            scroll: Default::default(), text_layouts: Vec::new(),
        }
    }

    #[test]
    fn update_writes_opacity_override_per_tick() {
        let mut s = one_node_scene();
        let mut mgr = TweenManager::new();
        mgr.tween(NodeId(0), TweenProp::Opacity, [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
                  Ease::Linear, 0.0, 1.0, 42);
        let mut out = Vec::new();
        // dt=0.5 → norm=0.5 → opacity=0.5
        mgr.update(0.5, &mut s, &mut out);
        assert!((s.anim.0[0].opacity.unwrap() - 0.5).abs() < 1e-5, "半程 opacity=0.5");
        assert!(out.is_empty(), "未结束 → 无 complete 事件");
    }

    #[test]
    fn update_emits_complete_with_tag_and_prop() {
        let mut s = one_node_scene();
        let mut mgr = TweenManager::new();
        mgr.tween(NodeId(0), TweenProp::Scale, [1.0, 1.0, 0.0, 0.0], [2.0, 3.0, 0.0, 0.0],
                  Ease::Linear, 0.0, 1.0, 7);
        let mut out = Vec::new();
        mgr.update(1.0, &mut s, &mut out);   // 恰好结束
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].event_type, EVT_TWEEN_COMPLETE);
        assert_eq!(out[0].click_count, TweenProp::Scale as u8, "click_count 复用装 prop");
        assert_eq!(out[0].touch_id, 7, "touch_id 复用装 tag");
        // 末值 = scale(2,3)
        let m = s.anim.0[0].transform.unwrap();
        assert!((m[0] - 2.0).abs() < 1e-5 && (m[3] - 3.0).abs() < 1e-5, "末值 scale(2,3)");
        // 完成后 tween 移除
        let mut out2 = Vec::new();
        mgr.update(1.0, &mut s, &mut out2);
        assert!(out2.is_empty(), "完成后不再产事件");
    }

    #[test]
    fn update_delay_gates_apply() {
        let mut s = one_node_scene();
        let mut mgr = TweenManager::new();
        mgr.tween(NodeId(0), TweenProp::Opacity, [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
                  Ease::Linear, 1.0, 1.0, 0);  // delay=1
        let mut out = Vec::new();
        mgr.update(0.5, &mut s, &mut out);   // elapsed 0.5 < delay 1 → 不写
        assert!(s.anim.0[0].opacity.is_none(), "delay 内不写 override");
        assert!(out.is_empty());
        mgr.update(1.0, &mut s, &mut out);   // elapsed 1.5，tt=0.5 → norm=0.5
        assert!((s.anim.0[0].opacity.unwrap() - 0.5).abs() < 1e-5, "越过 delay 后按 tt 插值");
    }

    #[test]
    fn kill_stops_update_keeps_override() {
        let mut s = one_node_scene();
        let mut mgr = TweenManager::new();
        mgr.tween(NodeId(0), TweenProp::Opacity, [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
                  Ease::Linear, 0.0, 1.0, 0);
        let mut out = Vec::new();
        mgr.update(0.3, &mut s, &mut out);
        let v = s.anim.0[0].opacity.unwrap();
        mgr.kill(NodeId(0), TweenProp::Opacity);
        mgr.update(0.5, &mut s, &mut out);   // kill 后不再推进
        assert_eq!(s.anim.0[0].opacity.unwrap(), v, "kill 后 override 保留末值不变");
        assert!(out.is_empty(), "kill 不产 complete");
    }

    #[test]
    fn update_skips_out_of_range_node() {
        let mut s = one_node_scene();   // 仅 node 0
        let mut mgr = TweenManager::new();
        mgr.tween(NodeId(5), TweenProp::Opacity, [0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0],
                  Ease::Linear, 0.0, 1.0, 0);
        let mut out = Vec::new();
        mgr.update(1.0, &mut s, &mut out);   // node 5 越界 → 跳过
        assert!(out.is_empty(), "越界 node 不产事件");
    }
}
