//! ScrollPane 状态 + 物理。transient（不进 pkg）。
//!
//! 本模块持数据模型：
//! - `ScrollPaneState`：每滚动容器几何（content/viewport/overlap）+ 物理状态（pos/velocity/tween）。
//! - `ScrollTable`：per-node 槽（`Vec<Option<ScrollPaneState>>`，NodeId 索引），镜像 `AnimTable` 模式。
//! - `refresh_content_sizes(&mut Scene)`：layout solve 后填 content_size/viewport/overlap。
//! - `capable` / `effective` helper。
//!
//! core 无 Vec2 类型——几何用 `(f32, f32)` 元组（照 `transform::apply_point`）。

use crate::scene::node::{NodeId, Node, Rect, Scene};
use crate::style::resolved::OverflowMode;

/// 滚轮输入事件（FFI POD）。C# set_wheel_input 推一组；core apply_wheel_to_hit
/// 沿祖先找最近 effective 滚动容器 → apply_wheel。
/// 16B：x@0 + y@4 + delta_x@8 + delta_y@12（4×f32 紧凑 ABI 断言）。
/// （x,y)=指针 design 坐标（hit_test 用）；(delta_x,delta_y)=滚轮增量（apply_wheel 吃）。
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct WheelEvent {
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
}
const _: () = {
    assert!(std::mem::size_of::<WheelEvent>() == 16);
}; // ABI 断言

// ── 物理常量 ─────────────────────────────────────────────
// 滚动触发阈值（px）：鼠标/触摸移动超此才认拖拽。
// mouse 8 / touch 20。
pub const SCROLL_THRESHOLD_MOUSE: f32 = 8.0;
pub const SCROLL_THRESHOLD_TOUCH: f32 = 20.0;
/// 惯性减速系数（每 1/60s 速度衰减比）。
pub const DECELERATION_RATE: f64 = 0.967;
/// 速度指数平滑系数（drag_follow 写 velocity 用）。
pub const VELOCITY_SMOOTH: f32 = 10.0;
/// 惯性触发阈值（px/s 线性 |v|）：PC。判 `|v|*scale < thresh`。
pub const INERTIA_THRESH_PC: f32 = 500.0;
/// 惯性触发阈值（px/s 线性 |v|）：触摸。
pub const INERTIA_THRESH_TOUCH: f32 = 1000.0;
/// 惯性位移系数（change = v*dur*0.4）。
pub const INERTIA_DIST_COEFF: f32 = 0.4;
/// 默认补间时长（s）：set_pos/wheel/bounce。
pub const TWEEN_TIME_DEFAULT: f32 = 0.3;
/// 越界打折比（drag_follow 越界位移 × 0.5）。
pub const PULL_RATIO: f32 = 0.5;
/// 回弹触发阈值（越界 abs > 20 才回弹，否则 snap）。
pub const BOUNCE_THRESHOLD: f32 = 20.0;
/// 滚轮步进（每 delta 单位位移 px）。
pub const SCROLL_STEP: f32 = 25.0;
/// scrollbar 轨道厚度（px）。
pub const SCROLLBAR_TRACK_THICKNESS: f32 = 8.0;
/// scrollbar thumb 最小尺寸（px，防 content 过长时 thumb 缩到不可见）。
pub const MIN_THUMB_SIZE: f32 = 20.0;

/// 合成 scrollbar thumb 的 sentinel node_id flag。
/// 合成 RenderNode 的 node_id = container_id.0 as u32 | flag（高位，真实 NodeId 小，复用稳定）。
pub const V_THUMB_FLAG: u32 = 0x4000_0000;
pub const H_THUMB_FLAG: u32 = 0x2000_0000;

/// cubic-out 缓动：(t-1)^3 + 1，t∈[0,1]。advance tween 用。
fn cubic_out(t: f32) -> f32 {
    let u = t - 1.0;
    u * u * u + 1.0
}

/// 单滚动容器状态。`#[derive(Default)]`：几何全 0、物理全 0/false、tweening=0（无）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollPaneState {
    /// 直接子 layout_rect 的 AABB 尺寸。
    pub content_size: (f32, f32),
    /// 本容器 content box 尺寸（layout_rect border box；padding 简化）。
    pub viewport_size: (f32, f32),
    /// (content - viewport).max(0) 每轴；负钳 0。
    pub overlap: (f32, f32),
    /// 当前滚动位置（content 坐标系偏移）。
    pub scroll_pos: (f32, f32),
    /// 惯性速度（px/s）。advance 写。
    pub velocity: (f32, f32),
    /// 0=无补间，1=set_pos 补间，2=惯性+回弹补间。advance 写。
    pub tweening: u8,
    pub tween_start: (f32, f32),
    pub tween_change: (f32, f32),
    pub tween_time: (f32, f32),
    pub tween_duration: (f32, f32),
    /// refresh 后若 content_size 变化置 true（供 scrollbar 复布局用）。
    pub content_size_dirty: bool,
}

/// 每节点滚动状态表（`HashMap<NodeId, ScrollPaneState>`）。仅滚动容器 ensure 后有值。
/// transient——不进 pkg（同 `anim` / `world_transforms`）。
///
/// **T3 校准**：原 `Vec<Option<ScrollPaneState>>`（按 id.index() 索引）改 HashMap（同 AnimTable，
/// 见 node.rs AnimTable doc：slotmap Key trait sealed，NodeId 不能直接当 SecondaryMap Key）。
#[derive(Debug, Clone, Default)]
pub struct ScrollTable(pub std::collections::HashMap<NodeId, ScrollPaneState>);

impl ScrollTable {
    pub fn get(&self, id: NodeId) -> Option<&ScrollPaneState> {
        self.0.get(&id)
    }
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut ScrollPaneState> {
        self.0.get_mut(&id)
    }
    /// 确保该节点有 scroll 槽并返回可变状态（缺则插 default）。
    pub fn ensure(&mut self, id: NodeId) -> &mut ScrollPaneState {
        self.0.entry(id).or_insert_with(ScrollPaneState::default)
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// 物理方法。per-axis 用 ax 0/1 分支，自维护可变 target tween
/// （不走 GTween）。tweening：0=无，1=set_pos/wheel，2=inertia/bounce。
impl ScrollPaneState {
    /// 拖拽跟手：scroll_pos += delta（越界 PULL_RATIO 打折）+ 记速度（exp 平滑）。
    pub fn drag_follow(&mut self, delta: (f32, f32), dt: f32) {
        // 速度记录（指数平滑：v += (Δ/dt - v) * smooth）
        if dt > 0.0 {
            let smoothing = (dt * VELOCITY_SMOOTH).clamp(0.0, 1.0);
            self.velocity.0 += (delta.0 / dt - self.velocity.0) * smoothing;
            self.velocity.1 += (delta.1 / dt - self.velocity.1) * smoothing;
        }
        for ax in 0..2u8 {
            let cur = if ax == 0 { self.scroll_pos.0 } else { self.scroll_pos.1 };
            let d = if ax == 0 { delta.0 } else { delta.1 };
            let ov = if ax == 0 { self.overlap.0 } else { self.overlap.1 };
            if ov <= 0.0 { continue; }  // 无 overlap 轴不动（防 overflow-y 容器斜拖 x 抖）
            let vp = if ax == 0 { self.viewport_size.0 } else { self.viewport_size.1 };
            let mut np = cur + d;
            let lo = 0.0f32;
            let hi = ov;
            if np < lo {
                // 越界单打折（min(位移*PULL_RATIO, vp*PULL_RATIO)）：最大越界 vp*PULL_RATIO。
                let dampened = ((lo - np) * PULL_RATIO).min(vp * PULL_RATIO);
                np = lo - dampened;
            } else if np > hi {
                let dampened = ((np - hi) * PULL_RATIO).min(vp * PULL_RATIO);
                np = hi + dampened;
            }
            if ax == 0 {
                self.scroll_pos.0 = np;
            } else {
                self.scroll_pos.1 = np;
            }
        }
        self.tweening = 0; // 拖拽中无 tween
    }

    /// Up 后松手物理（启 tween→tweening=2，否则 0）。is_touch 选阈值。
    /// 1. **越界**（start<0 或 >overlap）→ 直接 bounce 回边界（不论 velocity，不 inertia）。
    /// 2. **界内 + ratio>0** → inertia tween（target **不 clamp**，越界由 advance 运行时截断）。
    /// 3. **界内 + 低速（v2≤thresh）** → ratio=0，不设 tween（停）。
    ///
    /// v2 = |v|*scale（scale 默认 1）= 线性 |v|（"v2" 误导成 v²）；
    /// dur = |log(60/v2_eff)/log(DECELERATION_RATE)|/60。
    ///
    /// 手感对齐：① 越界松手 → bounce（不 snap）；② 二次 ratio `((v2-thresh)/thresh)²`
    /// 削弱低速；③ inertia target 不 clamp，advance 运行时 >20px 截断 + 回弹（弹性过冲回弹）。
    pub fn begin_inertia(&mut self, is_touch: bool) {
        let thresh = if is_touch { INERTIA_THRESH_TOUCH } else { INERTIA_THRESH_PC };
        let mut any = false;
        for ax in 0..2u8 {
            let v = if ax == 0 { self.velocity.0 } else { self.velocity.1 };
            let ov = if ax == 0 { self.overlap.0 } else { self.overlap.1 };
            let start = if ax == 0 { self.scroll_pos.0 } else { self.scroll_pos.1 };
            // 分支 1：越界 → bounce 回边界（越界必平滑回弹，不 snap）
            let over_lo = start < 0.0;
            let over_hi = ov > 0.0 && start > ov;
            if over_lo || over_hi {
                let boundary = if over_lo { 0.0 } else { ov };
                let change = boundary - start;
                if ax == 0 {
                    self.tween_start.0 = start;
                    self.tween_change.0 = change;
                    self.tween_duration.0 = TWEEN_TIME_DEFAULT;
                    self.tween_time.0 = 0.0;
                } else {
                    self.tween_start.1 = start;
                    self.tween_change.1 = change;
                    self.tween_duration.1 = TWEEN_TIME_DEFAULT;
                    self.tween_time.1 = 0.0;
                }
                any = true;
                continue;
            }
            // 分支 3：界内低速或无 overlap → 停（ratio=0 不 inertia）
            let v2 = v.abs();  // v2 = |v|*scale = 线性 |v|（非 v²）
            if ov <= 0.0 || v2 <= thresh {
                continue;
            }
            // 二次 ratio 削弱低速：ratio = ((v2-thresh)/thresh)²，clamp ≤1；v2 与 v 同乘 ratio。
            let ratio = (((v2 - thresh) / thresh).powi(2)).min(1.0);
            let v2_eff = v2 * ratio;
            let v_eff = v * ratio;
            // 分支 2：界内 inertia。dur = |log(60/v2_eff)/log(DECEL)|/60。
            // change = v_eff*dur*0.4（经验公式）。**不 clamp target**——越界由 advance
            // 运行时截断（>20px 越界 → 截断 + 回弹 = 弹性过冲回弹）。
            let dur = ((60.0f64 / v2_eff as f64).log(DECELERATION_RATE).abs() / 60.0) as f32;
            let dur = dur.max(TWEEN_TIME_DEFAULT);
            let change = v_eff * dur * INERTIA_DIST_COEFF;
            if ax == 0 {
                self.tween_start.0 = start;
                self.tween_change.0 = change;
                self.tween_duration.0 = dur;
                self.tween_time.0 = 0.0;
            } else {
                self.tween_start.1 = start;
                self.tween_change.1 = change;
                self.tween_duration.1 = dur;
                self.tween_time.1 = 0.0;
            }
            any = true;
        }
        self.tweening = if any { 2 } else { 0 };
    }

    /// 回弹 tween（越界 > BOUNCE_THRESHOLD 才启，否则 snap 由 advance done 处理）。
    pub fn begin_bounce(&mut self) {
        for ax in 0..2u8 {
            let cur = if ax == 0 { self.scroll_pos.0 } else { self.scroll_pos.1 };
            let ov = if ax == 0 { self.overlap.0 } else { self.overlap.1 };
            // 界内或小越界 → 不回弹
            let boundary = if cur < 0.0 {
                0.0
            } else if cur > ov {
                ov
            } else {
                continue;
            };
            if (cur - boundary).abs() < BOUNCE_THRESHOLD {
                continue;
            }
            let start = cur;
            let change = boundary - cur;
            if ax == 0 {
                self.tween_start.0 = start;
                self.tween_change.0 = change;
                self.tween_duration.0 = TWEEN_TIME_DEFAULT;
                self.tween_time.0 = 0.0;
            } else {
                self.tween_start.1 = start;
                self.tween_change.1 = change;
                self.tween_duration.1 = TWEEN_TIME_DEFAULT;
                self.tween_time.1 = 0.0;
            }
        }
        self.tweening = 2;
    }

    /// 推进 tween（tweening≠0）。
    /// 每帧 cubic_out 推进 pos；**tweening==2 时运行时检测越界**——pos 越界 >20px（或
    /// inertia 完成 change==0 时仍越界）即截断当前 tween，启回弹 tween（弹性过冲回弹）。
    /// 两轴 tween_change 都归零 → done（clamp[0,overlap] + tweening=0）。
    pub fn advance(&mut self, dt: f32) {
        if self.tweening == 0 {
            return;
        }
        for ax in 0..2u8 {
            let dur = if ax == 0 { self.tween_duration.0 } else { self.tween_duration.1 };
            if dur <= 0.0 {
                continue;
            }
            let change = if ax == 0 { self.tween_change.0 } else { self.tween_change.1 };
            if change == 0.0 {
                continue; // 该轴 tween 已完成（change 归零），待 done
            }
            let start = if ax == 0 { self.tween_start.0 } else { self.tween_start.1 };
            let ov = if ax == 0 { self.overlap.0 } else { self.overlap.1 };
            // 推进
            if ax == 0 {
                self.tween_time.0 += dt;
            } else {
                self.tween_time.1 += dt;
            }
            let t = if ax == 0 { self.tween_time.0 } else { self.tween_time.1 };
            let pos = if t >= dur {
                let p = start + change;
                if ax == 0 {
                    self.tween_change.0 = 0.0;
                } else {
                    self.tween_change.1 = 0.0;
                }
                p
            } else {
                start + change * cubic_out(t / dur)
            };
            if ax == 0 {
                self.scroll_pos.0 = pos;
            } else {
                self.scroll_pos.1 = pos;
            }
            // 运行时越界截断（仅 tweening==2）。
            // 越顶（pos<0）：inertia 往顶（cc<0）冲过 0 超 20px，或完成（cc==0）时仍越顶 → 回弹到 0。
            // 越底（pos>ov）：对称。→ 弹性过冲回弹（不冲远空白再突然 snap）。
            if self.tweening == 2 {
                let cc = if ax == 0 { self.tween_change.0 } else { self.tween_change.1 };
                let bounce = if (pos < -BOUNCE_THRESHOLD && cc < 0.0)
                    || (pos < 0.0 && cc == 0.0)
                {
                    Some((0.0_f32, 0.0 - pos))
                } else if ov > 0.0
                    && ((pos > ov + BOUNCE_THRESHOLD && cc > 0.0) || (pos > ov && cc == 0.0))
                {
                    Some((ov, ov - pos))
                } else {
                    None
                };
                if let Some((_boundary, new_change)) = bounce {
                    if ax == 0 {
                        self.tween_start.0 = pos;
                        self.tween_change.0 = new_change;
                        self.tween_duration.0 = TWEEN_TIME_DEFAULT;
                        self.tween_time.0 = 0.0;
                    } else {
                        self.tween_start.1 = pos;
                        self.tween_change.1 = new_change;
                        self.tween_duration.1 = TWEEN_TIME_DEFAULT;
                        self.tween_time.1 = 0.0;
                    }
                }
            }
        }
        // done：两轴 tween_change 都归零
        if self.tween_change.0 == 0.0 && self.tween_change.1 == 0.0 {
            self.scroll_pos.0 = self.scroll_pos.0.clamp(0.0, self.overlap.0);
            self.scroll_pos.1 = self.scroll_pos.1.clamp(0.0, self.overlap.1);
            self.tweening = 0;
        }
    }

    /// 滚轮：target = (cur - delta*SCROLL_STEP).clamp[0,overlap]，启 tweening=1。
    /// delta.y > 0 = 上滚（看上方）→ scroll_pos.y 减少。
    pub fn apply_wheel(&mut self, delta: (f32, f32)) {
        for ax in 0..2u8 {
            let d = if ax == 0 { delta.0 } else { delta.1 };
            if d == 0.0 {
                continue;
            }
            let cur = if ax == 0 { self.scroll_pos.0 } else { self.scroll_pos.1 };
            let ov = if ax == 0 { self.overlap.0 } else { self.overlap.1 };
            let target = (cur - d * SCROLL_STEP).clamp(0.0, ov);
            let start = cur;
            if ax == 0 {
                self.tween_start.0 = start;
                self.tween_change.0 = target - start;
                self.tween_duration.0 = TWEEN_TIME_DEFAULT;
                self.tween_time.0 = 0.0;
            } else {
                self.tween_start.1 = start;
                self.tween_change.1 = target - start;
                self.tween_duration.1 = TWEEN_TIME_DEFAULT;
                self.tween_time.1 = 0.0;
            }
        }
        self.tweening = 1;
    }

    /// 编程滚动。animated=false 直接 snap+clamp+tweening=0；true 启 tweening=1。
    pub fn set_pos(&mut self, target: (f32, f32), animated: bool) {
        if !animated {
            self.scroll_pos =
                (target.0.clamp(0.0, self.overlap.0), target.1.clamp(0.0, self.overlap.1));
            self.tweening = 0;
            return;
        }
        for ax in 0..2u8 {
            let t = if ax == 0 {
                target.0.clamp(0.0, self.overlap.0)
            } else {
                target.1.clamp(0.0, self.overlap.1)
            };
            let start = if ax == 0 { self.scroll_pos.0 } else { self.scroll_pos.1 };
            if ax == 0 {
                self.tween_start.0 = start;
                self.tween_change.0 = t - start;
                self.tween_duration.0 = TWEEN_TIME_DEFAULT;
                self.tween_time.0 = 0.0;
            } else {
                self.tween_start.1 = start;
                self.tween_change.1 = t - start;
                self.tween_duration.1 = TWEEN_TIME_DEFAULT;
                self.tween_time.1 = 0.0;
            }
        }
        self.tweening = 1;
    }
}

/// 该轴是否允许滚动（overflow ∈ {Scroll, Auto}）。
pub fn capable(ovf: OverflowMode) -> bool {
    matches!(ovf, OverflowMode::Scroll | OverflowMode::Auto)
}

/// 该轴实际可滚（capable 且 (Scroll 或 content > viewport)）。
/// Auto 仅当内容溢出才可滚；Scroll 无论溢出与否皆可滚。
pub fn effective(ovf: OverflowMode, content: f32, viewport: f32) -> bool {
    capable(ovf) && (ovf == OverflowMode::Scroll || content > viewport)
}

/// 垂直 thumb design-rect（容器 viewport 右边缘 track；thumb 大小/位置）。
/// 返 None 若 overlap_y <= 0（无溢出、无需 thumb）。
pub fn v_thumb_rect(scene: &Scene, id: NodeId) -> Option<Rect> {
    let s = scene.scroll.get(id)?;
    if s.overlap.1 <= 0.0 {
        return None;
    }
    let lr = scene.get(id).expect("live node").layout_rect;
    let track_w = SCROLLBAR_TRACK_THICKNESS;
    let track_h = lr.h;
    let thumb_h = (s.viewport_size.1 * (s.viewport_size.1 / s.content_size.1))
        .max(MIN_THUMB_SIZE)
        .min(track_h);
    let perc = if s.overlap.1 > 0.0 {
        (s.scroll_pos.1 / s.overlap.1).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thumb_y = lr.y + (track_h - thumb_h) * perc;
    Some(Rect {
        x: lr.x + lr.w - track_w,
        y: thumb_y,
        w: track_w,
        h: thumb_h,
    })
}

/// 水平 thumb design-rect（容器 viewport 底边 track；thumb 大小/位置）。
pub fn h_thumb_rect(scene: &Scene, id: NodeId) -> Option<Rect> {
    let s = scene.scroll.get(id)?;
    if s.overlap.0 <= 0.0 {
        return None;
    }
    let lr = scene.get(id).expect("live node").layout_rect;
    let track_h = SCROLLBAR_TRACK_THICKNESS;
    let track_w = lr.w;
    let thumb_w = (s.viewport_size.0 * (s.viewport_size.0 / s.content_size.0))
        .max(MIN_THUMB_SIZE)
        .min(track_w);
    let perc = if s.overlap.0 > 0.0 {
        (s.scroll_pos.0 / s.overlap.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thumb_x = lr.x + (track_w - thumb_w) * perc;
    Some(Rect {
        x: thumb_x,
        y: lr.y + lr.h - track_h,
        w: thumb_w,
        h: track_h,
    })
}

/// solve 后填 content_size/viewport/overlap。
/// 遍历节点：任一轴 overflow != Visible 即视为滚动容器，ensure 后写几何。
/// children clone 避借用冲突（遍历子 layout_rect 时也要借 scene.nodes）。
pub fn refresh_content_sizes(scene: &mut Scene) {
    // 收集 (nid, kids, viewport) 避免在借 scene.scroll 时再借 scene.nodes。
    let mut work: Vec<(NodeId, Vec<NodeId>, (f32, f32))> = Vec::new();
    for n in scene.nodes.values() {
        if n.style.overflow_x != OverflowMode::Visible
            || n.style.overflow_y != OverflowMode::Visible
        {
            let kids = n.children.clone();
            let viewport = content_box_size(n);
            work.push((n.id, kids, viewport));
        }
    }
    for (nid, kids, viewport) in work {
        // content_size = 直接子节点 layout_rect AABB。
        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for c in &kids {
            let r = scene.get(*c).expect("live node").layout_rect;
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
        let st = scene.scroll.ensure(nid);
        st.content_size_dirty = st.content_size != content;
        st.content_size = content;
        st.viewport_size = viewport;
        let new_overlap = (
            (content.0 - viewport.0).max(0.0),
            (content.1 - viewport.1).max(0.0),
        );
        st.overlap = new_overlap;
        // content_size 变化补偿（最小）：geometry 变了，若 scroll_pos 跑出新
        // [0, overlap] 范围，直接 clamp 并取消正在跑的 tween。
        // 完整补偿（按比例缩短 tween_change/tween_duration）defer——简化为 snap。
        if st.content_size_dirty && st.tweening != 0 {
            let out_of_range = st.scroll_pos.0 < 0.0
                || st.scroll_pos.0 > new_overlap.0
                || st.scroll_pos.1 < 0.0
                || st.scroll_pos.1 > new_overlap.1;
            if out_of_range {
                st.scroll_pos.0 = st.scroll_pos.0.clamp(0.0, new_overlap.0);
                st.scroll_pos.1 = st.scroll_pos.1.clamp(0.0, new_overlap.1);
                st.tweening = 0;
            }
        }
    }
}

/// content box 尺寸。简化：用 border box（layout_rect 尺寸）。
/// 已声明 padding 简化（建议 scroll 容器 padding:0）；padding 边缘处理 defer。
fn content_box_size(node: &Node) -> (f32, f32) {
    let lr = node.layout_rect;
    (lr.w, lr.h)
}

/// hit(x,y) → 沿 node.parent 链找最近 effective 滚动容器 → apply_wheel。
/// 无祖先（或无 effective）→ 丢弃（return）。effective 判定用 scene.scroll.get 取
/// content/viewport（无 state 视 0.0，effective 对 Scroll overflow 仍 true）。
pub fn apply_wheel_to_hit(scene: &mut Scene, w: WheelEvent) {
    let mut pane = crate::hit::hit_test(scene, (w.x, w.y));
    while let Some(id) = pane {
        // sentinel thumb_id → decode container_id（thumb covers container edge,
        // wheel on thumb = wheel on container）
        let id = if id.0 & 0x6000_0000 != 0 { NodeId(id.0 & !0x6000_0000) } else { id };
        if let Some(n) = scene.get(id) {
            let eff_y = effective(
                n.style.overflow_y,
                scene.scroll.get(id).map_or(0.0, |s| s.content_size.1),
                scene.scroll.get(id).map_or(0.0, |s| s.viewport_size.1),
            );
            let eff_x = effective(
                n.style.overflow_x,
                scene.scroll.get(id).map_or(0.0, |s| s.content_size.0),
                scene.scroll.get(id).map_or(0.0, |s| s.viewport_size.0),
            );
            if eff_y || eff_x {
                if let Some(s) = scene.scroll.get_mut(id) {
                    s.apply_wheel((w.delta_x, w.delta_y));
                }
                return;
            }
        } else {
            // defensive: invalid node id (shouldn't happen after sentinel decode)
            break;
        }
        pane = scene.get(id).expect("live node").parent;
    }
}

/// tick 推进所有活跃 scroll tween（tweening≠0）。
/// 遍历 scene.scroll（HashMap values_mut），每个 st 若 tweening≠0 调 st.advance(dt)。
/// tweening=0 的拖拽中/静止容器不 advance。
pub fn advance_all(dt: f32, scene: &mut Scene) {
    for st in scene.scroll.0.values_mut() {
        if st.tweening != 0 {
            st.advance(dt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::{NodeKind, Rect};
    use crate::style::resolved::ResolvedStyle;

    /// 构造滚动测试场景：
    ///   root0 = scroll 容器（overflow_y=Scroll），layout_rect (0,0,100,100)
    ///   child1 = root0 子，layout_rect (0,0,40,40)
    ///   child2 = root0 子，layout_rect (0,50,30,30)
    ///   root1 = 非 scroll（overflow 双轴 Visible），layout_rect (0,0,50,50)
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
        // root0 = scroll 容器（roots[0]）；root1 = 非 scroll（roots[1]）。
        let root0 = s.roots[0];
        let root1 = s.roots[1];
        let (c0, c1) = {
            let n = s.get(root0).unwrap();
            (n.children[0], n.children[1])
        };
        s.get_mut(root0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        s.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 50.0, w: 30.0, h: 30.0 };
        s.get_mut(root1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 };
        s
    }

    /// 取 scroll 容器 id（= roots[0]）。
    fn scroll_container_id(s: &Scene) -> NodeId {
        s.roots[0]
    }
    /// 取 root0 的两个子 id。
    fn child_ids(s: &Scene) -> (NodeId, NodeId) {
        let n = s.get(s.roots[0]).unwrap();
        (n.children[0], n.children[1])
    }
    /// 取非 scroll 节点 id（= roots[1]）。
    fn non_scroll_id(s: &Scene) -> NodeId {
        s.roots[1]
    }

    #[test]
    fn content_size_is_children_aabb() {
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(root0).expect("scroll 容器有 state");
        assert!(
            (st.content_size.0 - 40.0).abs() < 1e-3 && (st.content_size.1 - 80.0).abs() < 1e-3,
            "content_size = (40, 80)，got {:?}",
            st.content_size
        );
    }

    #[test]
    fn viewport_and_overlap_from_geometry() {
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(root0).unwrap();
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
        let root0 = scroll_container_id(&s);
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(root0).unwrap();
        assert_eq!(st.overlap, (0.0, 0.0));
    }

    #[test]
    fn overlap_positive_when_content_exceeds_viewport() {
        // 改子 layout_rect 让 content > viewport y 轴
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        let (c0, c1) = child_ids(&s);
        s.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 };
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(root0).unwrap();
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
        let root1 = non_scroll_id(&s);
        refresh_content_sizes(&mut s);
        // root1 双轴 Visible → 非 scroll 容器 → scroll.get 返 None
        assert!(s.scroll.get(root1).is_none(), "非 scroll 节点无 state");
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
    fn scrolltable_hashmap_get_mut_ensure_clear() {
        // ScrollTable 用 HashMap<NodeId, ScrollPaneState>（T3）。NodeId 已 impl Hash+Eq，
        // 不依赖 slotmap 主表存不存在，故此单元测试可不经 Scene 直接造字面量 NodeId。
        let mk = |idx: u32| NodeId((idx << 12) | 1);
        let mut t = ScrollTable::default();
        assert!(t.get(mk(2)).is_none(), "空表 get → None");
        // ensure 插 default
        let st = t.ensure(mk(2));
        st.scroll_pos = (5.0, 7.0);
        assert_eq!(t.0.len(), 1, "ensure(mk(2)) → 1 个条目");
        let got = t.get(mk(2)).unwrap();
        assert_eq!(got.scroll_pos, (5.0, 7.0));
        // get_mut
        {
            let m = t.get_mut(mk(2)).unwrap();
            m.scroll_pos = (1.0, 2.0);
        }
        assert_eq!(t.get(mk(2)).unwrap().scroll_pos, (1.0, 2.0));
        // ensure 同 id 二次返同槽（不重置）
        let st2 = t.ensure(mk(2));
        assert_eq!(st2.scroll_pos, (1.0, 2.0), "二次 ensure 不重置已有值");
        // 不同 id → 不同槽
        t.ensure(mk(5)).scroll_pos = (9.0, 9.0);
        assert_eq!(t.0.len(), 2, "ensure 不同 id → 2 个条目");
        assert!(t.get(mk(5)).is_some());
        // 未 ensure 的 id → None
        assert!(t.get(mk(99)).is_none(), "未 ensure 的 id → None");
        // clear
        t.clear();
        assert!(t.0.is_empty(), "clear 清空");
        assert!(t.get(mk(2)).is_none(), "clear 后 get None");
    }

    #[test]
    fn content_size_dirty_flag_when_changes() {
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(root0).unwrap();
        // 首次：原 default (0,0) → (40,80) → dirty true
        assert!(st.content_size_dirty, "首次填入非零 content → dirty");
        // 再 refresh 一次（content 不变）→ dirty false
        refresh_content_sizes(&mut s);
        let st2 = s.scroll.get(root0).unwrap();
        assert!(!st2.content_size_dirty, "content 未变 → dirty false");
        // 改子尺寸 → dirty true
        let (_, c1) = child_ids(&s);
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 };
        refresh_content_sizes(&mut s);
        let st3 = s.scroll.get(root0).unwrap();
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
        let root0 = s.roots[0];
        s.get_mut(root0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        refresh_content_sizes(&mut s);
        let st = s.scroll.get(root0).unwrap();
        assert_eq!(st.content_size, (0.0, 0.0), "无子 content = (0,0)");
        assert_eq!(st.overlap, (0.0, 0.0));
    }

    // ── 物理方法测 ────────────────────────────────────────
    #[test]
    fn drag_follow_one_to_one_within_bounds() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 100.0);
        st.viewport_size = (100.0, 50.0);
        st.drag_follow((0.0, 10.0), 0.016); // delta (0,10) 界内 1:1
        assert!(
            (st.scroll_pos.1 - 10.0).abs() < 1e-2,
            "跟手 1:1 界内无打折，got {}",
            st.scroll_pos.1
        );
    }

    #[test]
    fn drag_follow_beyond_bound_damped_by_pull_ratio() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 100.0);
        // viewport 必须够大（vp*PULL_RATIO=50 > delta 30）才不被 cap，打折全额生效
        st.viewport_size = (100.0, 100.0);
        st.scroll_pos = (0.0, 0.0);
        st.drag_follow((0.0, -30.0), 0.016); // 往上越界 30
        // 越界打折：over=30，scroll_pos.y = 0 - 30*0.5 = -15（PULL_RATIO）
        assert!(
            (st.scroll_pos.1 - (-15.0)).abs() < 1e-1,
            "越界 PULL_RATIO 打折，got {}",
            st.scroll_pos.1
        );
    }

    #[test]
    fn drag_follow_skips_zero_overlap_axis() {
        // overflow-y 容器 x 轴 overlap=0 → drag 不动 x（防斜拖 x 抖动）。
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 100.0);  // x overlap=0（仅垂直可滚）
        st.viewport_size = (100.0, 100.0);
        st.drag_follow((50.0, 10.0), 0.016);  // x delta=50 但 overlap.x=0
        assert!(st.scroll_pos.0 == 0.0, "overlap=0 轴 drag 不动（防抖），got {}", st.scroll_pos.0);
        assert!((st.scroll_pos.1 - 10.0).abs() < 1e-2, "y 轴正常跟手，got {}", st.scroll_pos.1);
    }

    /// 大越界（|np|>vp）→ 最大越界 = vp*PULL_RATIO（min(位移*0.5, vp*0.5)）。
    #[test]
    fn drag_follow_large_over_bound_caps_at_vp_pull_ratio() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 100.0);
        st.viewport_size = (100.0, 100.0); // vp=100
        st.scroll_pos = (0.0, 0.0);
        st.drag_follow((0.0, -500.0), 0.016); // 巨大越界（远超 vp）
        // 最大越界 = vp*PULL_RATIO = 100*0.5 = 50
        assert!(
            (st.scroll_pos.1 - (-50.0)).abs() < 1e-1,
            "大越界 cap 在 vp*PULL_RATIO=-50，got {}",
            st.scroll_pos.1
        );
    }

    #[test]
    fn inertia_advances_toward_target_then_settles() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 1000.0);
        st.scroll_pos = (0.0, 0.0);
        st.velocity = (0.0, 2000.0); // |v|=2000 > PC 阈值 500
        st.begin_inertia(false); // is_touch=false (PC 阈值 500)
                                 // v2=|v|=2000 → dur=|log(60/2000)/log(0.967)|/60 ≈ 1.74s
                                 // change=2000·1.74·0.4≈1387px > overlap 1000 → clamp 到 1000
                                 // 1.74s @16ms ≈ 109 步，150 步覆盖 ~2.4s > dur
        for _ in 0..150 {
            st.advance(0.016);
            if st.tweening == 0 {
                break;
            }
        }
        assert!(st.scroll_pos.1 > 100.0, "惯性产生了位移，got {}", st.scroll_pos.1);
        assert_eq!(st.tweening, 0, "tween 完成归零");
    }

    #[test]
    fn bounce_returns_to_boundary() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 100.0);
        st.scroll_pos = (0.0, -30.0); // 越界 30 > 20 阈值
        st.begin_bounce();
        for _ in 0..60 {
            st.advance(0.016);
            if st.tweening == 0 {
                break;
            }
        }
        assert!(
            (st.scroll_pos.1 - 0.0).abs() < 1e-2,
            "回弹回边界 0，got {}",
            st.scroll_pos.1
        );
    }

    #[test]
    fn wheel_steps_and_clamps() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 1000.0);
        st.apply_wheel((0.0, 1.0)); // delta_y=1 上滚 → scroll 减
                                    // 上滚 = scroll_pos.y 减少；clamp 后启 tween
        assert!(
            st.tweening != 0 || st.scroll_pos.1 == 0.0,
            "wheel 启 tween 或 clamp 到 0，tweening={}, pos={}",
            st.tweening,
            st.scroll_pos.1
        );
    }

    #[test]
    fn set_pos_snap_when_not_animated() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 100.0);
        st.tweening = 2; // 原有 tween 进行中
        st.set_pos((0.0, 50.0), false);
        assert_eq!(st.scroll_pos.1, 50.0, "snap 直接到位");
        assert_eq!(st.tweening, 0, "animated=false tweening 归零");
    }

    #[test]
    fn set_pos_animated_starts_tween() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 100.0);
        st.scroll_pos = (0.0, 10.0);
        st.set_pos((0.0, 50.0), true);
        assert_eq!(st.tweening, 1, "animated=true 启 tweening=1");
        assert_eq!(st.tween_start.1, 10.0, "tween_start = 当前 pos");
        assert_eq!(st.tween_change.1, 40.0, "tween_change = target - start");
        assert_eq!(st.tween_duration.1, TWEEN_TIME_DEFAULT);
    }

    #[test]
    fn cubic_out_curve_endpoints() {
        assert!((cubic_out(0.0) - 0.0).abs() < 1e-4, "cubic_out(0)=0");
        assert!((cubic_out(1.0) - 1.0).abs() < 1e-4, "cubic_out(1)=1");
        // 单调增（中点 > 0.5，缓动尾部慢）
        let mid = cubic_out(0.5);
        assert!(mid > 0.5 && mid < 1.0, "cubic_out(0.5)∈(0.5,1)，got {}", mid);
    }

    // ── content_size 变化补偿（最小） ────────────────────────────────────
    #[test]
    fn content_size_change_clamps_running_tween() {
        // 滚动到 pos=80（overlap=100），tweening≠0；然后 content 缩 → overlap 变 50
        // → scroll_pos 越界（80 > 50）→ refresh 应 clamp + tweening 归零
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        let (c0, c1) = child_ids(&s);
        s.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 };
        refresh_content_sizes(&mut s);
        let st = s.scroll.get_mut(root0).unwrap();
        st.scroll_pos = (0.0, 80.0);
        st.tweening = 1; // 模拟 tween 进行中
                         // 缩 content：子 2 高度 200→100 → content_y=100，viewport=100 → overlap_y=0
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 100.0 };
        refresh_content_sizes(&mut s);
        let st2 = s.scroll.get(root0).unwrap();
        assert_eq!(st2.overlap.1, 0.0, "content 缩后 overlap=0");
        assert_eq!(st2.scroll_pos.1, 0.0, "越界 pos 被 clamp 到新 overlap");
        assert_eq!(st2.tweening, 0, "content 变化时 tween 取消");
    }

    #[test]
    fn content_size_change_in_range_keeps_tween() {
        // pos 在新 [0, overlap] 内 → 不打断 tween（最小补偿仅处理越界）
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        let (c0, c1) = child_ids(&s);
        s.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 };
        refresh_content_sizes(&mut s);
        let st = s.scroll.get_mut(root0).unwrap();
        st.scroll_pos = (0.0, 10.0);
        st.tweening = 1;
        // content 略缩但 pos=10 仍在 [0, overlap]（新 overlap 仍 ≥ 10）
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 150.0 };
        refresh_content_sizes(&mut s);
        let st2 = s.scroll.get(root0).unwrap();
        assert_eq!(st2.tweening, 1, "pos 在范围内不打断 tween");
    }

    // ── apply_wheel_to_hit ─────────────────────────────────────────────
    #[test]
    fn apply_wheel_to_hit_scrolls_nearest_effective_ancestor() {
        use crate::scene::transform::compute_world_transforms;

        // 构造 scene：overflow:scroll 容器 + content>viewport（effective_y=true）
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        let (_, c1) = child_ids(&s);
        // 扩子节点使 content_size > viewport_size on y 轴
        // content AABB y = max(40, 250) = 250 > viewport=100 → overlap_y=150
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 250.0 };
        // Scene::build 为 overflow node 设 clip_rect=Rect::default()（(0,0,0,0) 挡全部命中）；
        // 手填为 layout_rect 同尺寸让 hit_test 能命中。
        s.get_mut(root0).unwrap().clip_rect = Some(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });

        // 填 scroll state（content_size/viewport/overlap）+ world transforms（hit_test 用）
        refresh_content_sizes(&mut s);
        compute_world_transforms(&mut s);

        // 核实场景生效
        {
            let st = s.scroll.get(root0).unwrap();
            assert!(st.overlap.1 > 0.0, "content 超出 viewport，overlap_y={}", st.overlap.1);
            assert_eq!(st.tweening, 0, "初始 tweening=0");
        }

        // hit 容器内一点 (10,10) → hit_test 命中子节点 1 → parent 遍历到节点 0
        // → 节点 0 overflow_y=Scroll + effective → apply_wheel
        apply_wheel_to_hit(
            &mut s,
            WheelEvent { x: 10.0, y: 10.0, delta_x: 0.0, delta_y: 1.0 },
        );

        let st = s.scroll.get(root0).unwrap();
        assert!(st.tweening != 0, "wheel 触发滚动 tween，tweening={}", st.tweening);
    }

    /// wheel 落 thumb 区域，hit_test 返 sentinel → apply_wheel_to_hit 解码
    /// container_id 继续祖先链，不 crash 且正确滚该容器。
    #[test]
    fn apply_wheel_to_hit_on_thumb_decodes_sentinel() {
        use crate::scene::transform::compute_world_transforms;

        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        let (_, c1) = child_ids(&s);
        // content_y=250 > viewport=100 → overlap_y=150
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 250.0 };
        s.get_mut(root0).unwrap().clip_rect = Some(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });

        refresh_content_sizes(&mut s);
        compute_world_transforms(&mut s);

        // 核实 scroll state
        let st = s.scroll.get(root0).unwrap();
        assert!(st.overlap.1 > 0.0, "overlap needed for thumb");
        assert_eq!(st.tweening, 0);

        // v_thumb_rect: x=92, y=0, w=8, h=40（100*(100/250)=40）
        // 点 (96, 20) 在 thumb 内 → hit_test 应返 sentinel
        let hit = crate::hit::hit_test(&s, (96.0, 20.0));
        assert!(
            hit.map_or(false, |id| id.0 & 0x6000_0000 != 0),
            "thumb 命中应返 sentinel，got {:?}",
            hit
        );

        // apply_wheel_to_hit：sentinel 解码 → container 0 → apply_wheel
        apply_wheel_to_hit(
            &mut s,
            WheelEvent { x: 96.0, y: 20.0, delta_x: 0.0, delta_y: 1.0 },
        );

        let st = s.scroll.get(root0).unwrap();
        assert!(st.tweening != 0, "thumb wheel 应触发滚动，tweening={}", st.tweening);
    }

    // ── thumb rect 测 ─────────────────────────────────────────
    #[test]
    fn v_thumb_rect_is_right_edge_with_proportional_size() {
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        let (c0, c1) = child_ids(&s);
        s.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 };
        refresh_content_sizes(&mut s);
        // viewport=(100,100) content=(40,200) → overlap=(0,100)
        // thumb_h = 100*(100/200)=50, track_h=100, perc=0 → thumb_y = lr.y=0
        let r = v_thumb_rect(&s, root0).expect("overlap>0 → thumb");
        assert_eq!(r.w, 8.0, "track_w=8");
        assert!((r.h - 50.0).abs() < 1e-2, "thumb_h = 100*(100/200)=50, got {}", r.h);
        assert_eq!(r.x, 92.0, "右边缘: x = lr.x(0) + lr.w(100) - track_w(8)");
        assert_eq!(r.y, 0.0, "scroll_pos=0 → thumb 在顶端");
    }

    #[test]
    fn v_thumb_rect_moves_with_scroll_pos() {
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        let (c0, c1) = child_ids(&s);
        s.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 30.0, h: 200.0 };
        refresh_content_sizes(&mut s);
        let st = s.scroll.get_mut(root0).unwrap();
        st.scroll_pos.1 = 50.0; // 50% scrolled
        let r = v_thumb_rect(&s, root0).unwrap();
        // thumb_h=50, track_h=100, travel=50, perc=0.5 → thumb_y = 0 + 50*0.5 = 25
        assert!((r.y - 25.0).abs() < 1e-2, "50% scroll → thumb_y=25, got {}", r.y);
    }

    #[test]
    fn thumb_rect_returns_none_when_no_overlap() {
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        let (c0, c1) = child_ids(&s);
        // content < viewport → overlap=(0,0)
        s.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 40.0, h: 40.0 };
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 50.0, w: 30.0, h: 30.0 };
        refresh_content_sizes(&mut s);
        assert!(v_thumb_rect(&s, root0).is_none(), "overlap=0 → 无 thumb");
        assert!(h_thumb_rect(&s, root0).is_none(), "overlap=0 → 无 thumb");
    }

    #[test]
    fn h_thumb_rect_is_bottom_edge() {
        let mut s = build_scroll_scene();
        let root0 = scroll_container_id(&s);
        let (c0, c1) = child_ids(&s);
        s.get_mut(c0).unwrap().layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 40.0 };
        s.get_mut(c1).unwrap().layout_rect = Rect { x: 0.0, y: 50.0, w: 30.0, h: 30.0 };
        refresh_content_sizes(&mut s);
        // viewport=(100,100) content=(200,80) → overlap=(100,0)
        // h_thumb: track_h=8, track_w=100, thumb_w=100*(100/200)=50
        let r = h_thumb_rect(&s, root0).expect("overlap_x>0 → h_thumb");
        assert_eq!(r.h, 8.0, "track_h=8");
        assert!((r.w - 50.0).abs() < 1e-1, "thumb_w = 100*(100/200)=50");
        assert_eq!(r.y, 92.0, "底边: y = lr.y(0) + lr.h(100) - track_h(8)");
    }

    // ── 滚动松手物理 ─────────────────────────────────────────────────
    // 越界松手：直接 bounce 回边界，不 inertia。
    // 界内：二次 ratio 削弱低速；inertia target 不 clamp，advance 运行时越界 >20px 截断 + 回弹。
    /// 界内速度刚过阈值：二次 ratio 削弱使 change 极小（≈5px），而非全速冲越界。
    #[test]
    fn inertia_quad_ratio_damps_low_velocity() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 1000.0);
        st.viewport_size = (200.0, 200.0);
        st.scroll_pos = (0.0, 500.0);
        st.velocity = (0.0, 625.0); // 刚过 PC 阈值 500
        st.begin_inertia(false);
        assert_eq!(st.tweening, 2, "ratio>0 启 inertia");
        assert!(
            st.tween_change.1.abs() < 10.0,
            "二次 ratio 削弱：|change|<10（≈5px），got {}",
            st.tween_change.1
        );
    }

    /// 界内快速 inertia（target 远超 overlap）→ advance 运行时越界截断 + 回弹
    /// （弹性过冲），不冲远空白；最终回弹到边界。
    #[test]
    fn inertia_overshoot_then_bounce_back_to_boundary() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 400.0);
        st.viewport_size = (200.0, 200.0);
        st.scroll_pos = (0.0, 380.0);
        st.velocity = (0.0, 2000.0);
        st.begin_inertia(false);
        let mut max_pos: f32 = 0.0;
        let mut settled = false;
        for _ in 0..300 {
            st.advance(0.016);
            max_pos = max_pos.max(st.scroll_pos.1);
            if st.tweening == 0 {
                settled = true;
                break;
            }
        }
        assert!(settled, "inertia + 回弹应完成");
        assert!(
            (st.scroll_pos.1 - 400.0).abs() < 1e-1,
            "最终回弹到边界 400，got {}",
            st.scroll_pos.1
        );
        // 过冲有上限（运行时截断）
        assert!(
            max_pos < 500.0,
            "过冲 <500（弹性过冲上限），got {}",
            max_pos
        );
    }

    #[test]
    fn over_bounds_small_release_bounces_smoothly_not_snap() {
        // drag 越界 5px + 小 velocity（<PC 阈值 500）：松手应平滑 bounce 回边界，
        // 而非 advance done 瞬间 clamp snap。
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 400.0);
        st.viewport_size = (200.0, 200.0);
        st.scroll_pos = (0.0, -5.0);
        st.velocity = (0.0, -100.0);
        st.begin_inertia(false);
        assert_eq!(st.tweening, 2, "越界松手启 bounce tween");
        assert!(
            (st.tween_change.1 - 5.0).abs() < 1e-2,
            "bounce change = 0-(-5) = +5，got {}",
            st.tween_change.1
        );
        // 推进 1 帧：cubic_out(norm<<1) 平滑，pos 不应瞬间到 0（snap）
        st.advance(0.016);
        assert!(
            st.scroll_pos.1 > -5.0 && st.scroll_pos.1 < 0.0,
            "第 1 帧平滑回弹（非瞬间 snap），got {}",
            st.scroll_pos.1
        );
        for _ in 0..60 {
            st.advance(0.016);
            if st.tweening == 0 {
                break;
            }
        }
        assert!(
            (st.scroll_pos.1 - 0.0).abs() < 1e-2,
            "bounce 回边界 0，got {}",
            st.scroll_pos.1
        );
    }

    #[test]
    fn over_bounds_fast_velocity_bounces_not_overshoot() {
        // drag 越界 25px + 越界方向快速 velocity：松手应 bounce 回边界，
        // 不应 inertia 冲到巨量空白再 snap。
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 400.0);
        st.viewport_size = (200.0, 200.0);
        st.scroll_pos = (0.0, -25.0);
        st.velocity = (0.0, -2000.0);
        st.begin_inertia(false);
        for _ in 0..200 {
            st.advance(0.016);
            assert!(
                st.scroll_pos.1 >= -30.0,
                "越界松手不冲空白（>=-30），got {}",
                st.scroll_pos.1
            );
            if st.tweening == 0 {
                break;
            }
        }
        assert!(
            (st.scroll_pos.1 - 0.0).abs() < 1e-1,
            "bounce 回边界 0，got {}",
            st.scroll_pos.1
        );
    }

    /// 界内 velocity 不足松手 → 停在当前位置（不启 inertia 也不 bounce）。
    /// 界内慢拖松手不应回原位、不应弹。
    #[test]
    fn in_bounds_low_velocity_stays_put() {
        let mut st = ScrollPaneState::default();
        st.overlap = (0.0, 400.0);
        st.viewport_size = (200.0, 200.0);
        st.scroll_pos = (0.0, 100.0); // 界内中间
        st.velocity = (0.0, 100.0);   // <500 阈值
        st.begin_inertia(false);
        assert_eq!(st.tweening, 0, "界内 velocity 不足 → 不启 tween（停）");
        st.advance(0.016);
        assert!(
            (st.scroll_pos.1 - 100.0).abs() < 1e-4,
            "pos 保持 100（不回原位/不弹），got {}",
            st.scroll_pos.1
        );
    }
}
