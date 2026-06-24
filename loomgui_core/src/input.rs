//! 指针输入事件 + 多指针状态机（§10.3）。v1c.3：固定 5 槽（slot0=鼠标，slot1-4=触摸）。
//! 消费 PointerEvent[] + 命中 → 产 EventRecord[]。click 阈值 ~10px（§10.3 鼠标）。
//! disabled 节点产 RollOver/Out 但不产 Down/Up/Click。

use crate::hit::hit_test;
use crate::scene::node::{NodeId, Scene};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub button: u8,
    pub pad: [u8; 2],
    pub touch_id: i32,   // v1c.3：-1=鼠标主指 slots[0]；>=0=触摸 fingerId
    pub x: f32,
    pub y: f32,
}

/// 指针事件种类。repr(u8)：FFI 1 字节判别（PointerEvent 16B 紧凑布局，C# 对齐 byte）。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Down = 0,
    Up = 1,
    Move = 2,
}

/// 事件输出（FFI 扁平 POD）。event_type: 0=Down,1=Up,2=Move,3=Click,4=RollOver,5=RollOut。
/// v1c.3：+touch_id:i32 @8（破 v1c.2 零改，16→20 字节）。v1c.4：pad[0]→click_count（20B 不变）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EventRecord {
    pub node_id: u32,
    pub event_type: u8,
    pub click_count: u8,    // v1c.4：1 或 2（仅 Click 有意义，其余=0）
    pub pad: [u8; 2],
    pub touch_id: i32,
    pub x: f32,
    pub y: f32,
}

pub const EVT_DOWN: u8 = 0;
pub const EVT_UP: u8 = 1;
pub const EVT_MOVE: u8 = 2;
pub const EVT_CLICK: u8 = 3;
pub const EVT_ROLL_OVER: u8 = 4;
pub const EVT_ROLL_OUT: u8 = 5;

const CLICK_THRESHOLD_MOUSE: f32 = 10.0;   // fgui _clickTestThreshold(mouse)，per-axis
const CLICK_THRESHOLD_TOUCH: f32 = 50.0;   // fgui _clickTestThreshold(touch)
const DOUBLE_CLICK_TIME: f32 = 0.35;   // fgui 0.35f 秒
const MOVE_CANCEL_PX: f32 = 50.0;      // fgui Move 硬编码取消阈值（per-axis，mouse+touch 通用）

fn click_threshold(touch_id: i32) -> f32 {
    if touch_id == -1 { CLICK_THRESHOLD_MOUSE } else { CLICK_THRESHOLD_TOUCH }
}

/// 单触摸槽状态（v1c.3）。slots[0]=鼠标主指（touch_id=-1 常驻），slots[1..4]=触摸。
#[derive(Debug, Clone)]
pub struct TouchSlot {
    pub touch_id: i32,                  // -1=鼠标主指/空闲触摸槽；>=0=触摸 fingerId
    pub last_pos: (f32, f32),
    pub is_down: bool,
    pub down_node: Option<NodeId>,
    pub down_pos: (f32, f32),
    pub last_hit: Option<NodeId>,       // v1c.3：本帧命中（hover_diff + is_pointer_on_ui 用）
    pub last_hovered_chain: Vec<NodeId>,
    pub touch_monitors: Vec<NodeId>,    // v1c.3：capture 的节点（T2 填派发逻辑）
    pub down_targets: Vec<NodeId>,      // v1c.4：Down 时填 [leaf, …祖先]（照 fgui downTargets）
    pub click_cancelled: bool,          // v1c.4：Move>50 / CancelClick / Canceled 置
    pub last_click_time: f32,           // v1c.4：time_s（双击窗口）
    pub last_click_pos: (f32, f32),     // v1c.4：上次 Click 位置
    pub last_click_button: u8,          // v1c.4：上次 Click 键
    pub click_count: u8,                // v1c.4：1→2→1 循环
}

impl TouchSlot {
    fn new_mouse() -> Self {
        Self {
            touch_id: -1,
            last_pos: (0.0, 0.0),
            is_down: false,
            down_node: None,
            down_pos: (0.0, 0.0),
            last_hit: None,
            last_hovered_chain: Vec::new(),
            touch_monitors: Vec::new(),
            down_targets: Vec::new(),
            click_cancelled: false,
            last_click_time: 0.0,
            last_click_pos: (0.0, 0.0),
            last_click_button: 0,
            click_count: 1,
        }
    }
    fn new_free() -> Self {
        Self {
            touch_id: -1,
            last_pos: (0.0, 0.0),
            is_down: false,
            down_node: None,
            down_pos: (0.0, 0.0),
            last_hit: None,
            last_hovered_chain: Vec::new(),
            touch_monitors: Vec::new(),
            down_targets: Vec::new(),
            click_cancelled: false,
            last_click_time: 0.0,
            last_click_pos: (0.0, 0.0),
            last_click_button: 0,
            click_count: 1,
        }
    }
}

/// 多指针状态机（v1c.3：固定 5 槽）。slots[0]=鼠标，slots[1..4]=触摸。
pub struct PointerState {
    pub slots: Vec<TouchSlot>,
    pub time_s: f32,   // v1c.4：累积时间（Stage::advance_time 累加；双击窗口用）
}

impl Default for PointerState {
    fn default() -> Self {
        let mut slots = Vec::with_capacity(5);
        slots.push(TouchSlot::new_mouse());     // slot 0 = 鼠标主指
        for _ in 0..4 {
            slots.push(TouchSlot::new_free());  // slot 1..4 = 触摸
        }
        Self { slots, time_s: 0.0 }
    }
}

/// target 起沿 Node.parent 至 root 收集 NodeId 链（含 target）；target=None → 空链。
fn ancestor_chain(scene: &Scene, target: Option<NodeId>) -> Vec<NodeId> {
    let mut chain = Vec::new();
    let mut cur = target;
    while let Some(id) = cur {
        if id.0 >= scene.nodes.len() {
            break; // 防御（脏 scene）
        }
        chain.push(id);
        cur = scene.nodes[id.0].parent;
    }
    chain
}

impl PointerState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 鼠标主指 last_pos（stage.rs tick_and_render 的 hit_test 用，保持 v1c.2 接口）。
    pub fn last_pos(&self) -> (f32, f32) {
        self.slots[0].last_pos
    }

    /// 任一活跃槽命中非根节点 → UI 挡住（§10.6）。
    pub fn is_pointer_on_ui(&self, scene: &Scene) -> bool {
        let root_id = scene.roots.first().copied();
        for slot in &self.slots {
            if let Some(hit) = slot.last_hit {
                if Some(hit) != root_id {
                    return true;
                }
            }
        }
        false
    }

    /// 加 touch monitor（去重）。touch_id 找槽（鼠标=-1→slot0）；找不到槽→no-op（Down 前调无效）。
    /// 照 fgui AddTouchMonitor：仅加指定槽，不做 -1 广播（fgui 自身不用）。
    pub fn add_touch_monitor(&mut self, touch_id: i32, node: NodeId) {
        let slot_idx = if touch_id == -1 { 0 } else {
            match (1..self.slots.len()).find(|&i| self.slots[i].touch_id == touch_id) { Some(i) => i, None => return }
        };
        let slot = &mut self.slots[slot_idx];
        if !slot.touch_monitors.contains(&node) {
            slot.touch_monitors.push(node);
        }
    }

    /// 移除 touch monitor（从所有槽）。照 fgui RemoveTouchMonitor：置 sentinel 而非 RemoveAt（避免遍历偏移）。
    pub fn remove_touch_monitor(&mut self, node: NodeId) {
        for slot in &mut self.slots {
            // touch_monitors 是 Vec<NodeId>，用 retain 移除（Vec 无 sentinel 需求，retain 更简且无遍历期偏移）
            slot.touch_monitors.retain(|n| *n != node);
        }
    }

    /// 找/分配槽。鼠标(touch_id=-1)恒 slots[0]；触摸按 touch_id 找，找不到→分配首个空闲。
    /// 返回 slot index；找不到（触摸槽满）→ None。
    /// 注：触摸槽在任意事件（Move/Down/Up）分配（fgui 触摸可 Move 先于 Down 合成），
    /// Up 后释放（slot_idx>0 置 touch_id=-1）。
    fn find_or_alloc_slot(&mut self, ev: &PointerEvent) -> Option<usize> {
        if ev.touch_id == -1 {
            return Some(0); // 鼠标主指
        }
        // 找已占触摸槽
        for i in 1..self.slots.len() {
            if self.slots[i].touch_id == ev.touch_id {
                return Some(i);
            }
        }
        // 分配首个空闲触摸槽
        for i in 1..self.slots.len() {
            if self.slots[i].touch_id == -1 {
                self.slots[i].touch_id = ev.touch_id;
                return Some(i);
            }
        }
        None // 触摸槽满 → 丢弃
    }

    /// 消费本帧输入 → 产 EventRecord 序列。
    pub fn process(&mut self, scene: &mut Scene, events: &[PointerEvent]) -> Vec<EventRecord> {
        let mut out: Vec<EventRecord> = Vec::new();
        let time_s = self.time_s;   // 本地副本：避免事件循环内 &mut slot 与 &self.time_s 借用冲突（Up 臂用）
        if events.is_empty() {
            for i in 0..self.slots.len() {
                if i == 0 || self.slots[i].touch_id >= 0 {
                    // 活跃槽
                    Self::hover_diff_slot(&mut self.slots[i], scene, &mut out);
                }
            }
            self.recompute_hovered(scene);
            self.recompute_active(scene);
            return out;
        }
        for ev in events {
            let slot_idx = match self.find_or_alloc_slot(ev) {
                Some(i) => i,
                None => continue,
            };
            let slot = &mut self.slots[slot_idx];
            slot.last_pos = (ev.x, ev.y);
            let hit = hit_test(scene, slot.last_pos);
            slot.last_hit = hit;
            let touch_id = ev.touch_id;
            match ev.kind {
                PointerKind::Move => {
                    // v1c.4：按住中位移>50（per-axis，硬编码，mouse+touch 通用）→ 取消 click
                    if slot.is_down {
                        let dx = slot.last_pos.0 - slot.down_pos.0;
                        let dy = slot.last_pos.1 - slot.down_pos.1;
                        if dx.abs() > MOVE_CANCEL_PX || dy.abs() > MOVE_CANCEL_PX {
                            slot.click_cancelled = true;
                        }
                    }
                    Self::hover_diff_slot(slot, scene, &mut out);
                    // Move 派发：有 monitor 产 Move@monitor（T2 实现），无 monitor 不产
                    for m in &slot.touch_monitors {
                        out.push(EventRecord {
                            node_id: m.0 as u32,
                            event_type: EVT_MOVE,
                            click_count: 0,
                            pad: [0, 0],
                            touch_id,
                            x: ev.x,
                            y: ev.y,
                        });
                    }
                }
                PointerKind::Down => {
                    slot.is_down = true;
                    slot.down_pos = (ev.x, ev.y);
                    slot.down_node = hit;
                    slot.down_targets = ancestor_chain(scene, hit);   // v1c.4：[leaf,…祖先]
                    slot.click_cancelled = false;                     // 新按下重置
                    if let Some(n) = hit {
                        if !scene.nodes[n.0].disabled {
                            out.push(EventRecord {
                                node_id: n.0 as u32,
                                event_type: EVT_DOWN,
                                click_count: 0,
                                pad: [0, 0],
                                touch_id,
                                x: ev.x,
                                y: ev.y,
                            });
                        }
                    }
                    Self::hover_diff_slot(slot, scene, &mut out);
                }
                PointerKind::Up => {
                    slot.is_down = false;
                    if let Some(n) = hit {
                        if !scene.nodes[n.0].disabled {
                            out.push(EventRecord {
                                node_id: n.0 as u32,
                                event_type: EVT_UP,
                                click_count: 0,
                                pad: [0, 0],
                                touch_id,
                                x: ev.x,
                                y: ev.y,
                            });
                            if let Some(target) = Self::click_test(slot, scene, hit) {
                                if !scene.nodes[target.0].disabled {
                                    let count = Self::bump_click_count(slot, ev.button, time_s);
                                    out.push(EventRecord {
                                        node_id: target.0 as u32,
                                        event_type: EVT_CLICK,
                                        click_count: count,
                                        pad: [0, 0],
                                        touch_id,
                                        x: ev.x,
                                        y: ev.y,
                                    });
                                }
                            } else {
                                // click_test 返 None（位移超阈值/cancelled）→ 重置双击窗口（照 fgui End cancel 分支）
                                slot.last_click_time = 0.0;
                                slot.click_count = 1;
                            }
                        }
                    }
                    // monitor 的 Up 直派（去重：monitor != hit）
                    for m in &slot.touch_monitors {
                        if Some(*m) != hit {
                            out.push(EventRecord {
                                node_id: m.0 as u32,
                                event_type: EVT_UP,
                                click_count: 0,
                                pad: [0, 0],
                                touch_id,
                                x: ev.x,
                                y: ev.y,
                            });
                        }
                    }
                    slot.touch_monitors.clear();
                    slot.down_targets.clear();
                    slot.down_node = None;
                    Self::hover_diff_slot(slot, scene, &mut out);
                    if slot_idx > 0 {
                        slot.touch_id = -1; // 释放触摸槽（鼠标不释放）
                    }
                }
            }
        }
        self.recompute_hovered(scene);
        self.recompute_active(scene);
        out
    }

    /// click 目标判定（照 fgui ClickTest）。返 Click 应派发的节点；None=不产 Click。
    /// cancelled（Move>50/CancelClick/Canceled）→ None。位移 per-axis 超阈值 → None。
    /// 否则优先 down_targets[0]（按下叶，"still on stage"≈索引有效）；叶失效则沿当前 hit 祖先兜底。
    fn click_test(slot: &TouchSlot, scene: &Scene, current_hit: Option<NodeId>) -> Option<NodeId> {
        if slot.click_cancelled {
            return None;
        }
        let t = click_threshold(slot.touch_id);
        let dx = slot.last_pos.0 - slot.down_pos.0;
        let dy = slot.last_pos.1 - slot.down_pos.1;
        if dx.abs() > t || dy.abs() > t {
            return None;
        }
        if let Some(&leaf) = slot.down_targets.first() {
            if leaf.0 < scene.nodes.len() {
                return Some(leaf);
            }
        }
        let mut cur = current_hit;
        while let Some(id) = cur {
            if id.0 >= scene.nodes.len() {
                break;
            }
            if slot.down_targets.contains(&id) {
                return Some(id);
            }
            cur = scene.nodes[id.0].parent;
        }
        None
    }

    /// 双击 clickCount 累进（照 fgui End：350ms + per-axis 位置 + 同键 → 1→2→1 循环）。
    /// 返回本次 click_count 并更新 slot 的 last_click_* 状态。
    /// time_s 作参数传（非读 self.time_s），避免 &mut self 与 &mut slot 借用冲突。
    fn bump_click_count(slot: &mut TouchSlot, button: u8, time_s: f32) -> u8 {
        let t = click_threshold(slot.touch_id);
        let within_time = time_s - slot.last_click_time < DOUBLE_CLICK_TIME;
        let within_pos = (slot.last_pos.0 - slot.last_click_pos.0).abs() < t
            && (slot.last_pos.1 - slot.last_click_pos.1).abs() < t;
        let same_button = slot.last_click_button == button;
        let count = if within_time && within_pos && same_button {
            if slot.click_count == 2 { 1 } else { slot.click_count + 1 }   // 1→2→1 循环
        } else {
            1
        };
        slot.click_count = count;
        slot.last_click_time = time_s;
        slot.last_click_pos = slot.last_pos;
        slot.last_click_button = button;
        count
    }

    /// per-slot hover diff：产 RollOut(旧链独有)/RollOver(新链独有)。
    /// 不调 set_hovered_chain（全局 union 在 recompute_hovered）。
    fn hover_diff_slot(slot: &mut TouchSlot, scene: &mut Scene, out: &mut Vec<EventRecord>) {
        let new_chain = ancestor_chain(scene, slot.last_hit);
        if new_chain == slot.last_hovered_chain {
            return;
        }
        for n in &slot.last_hovered_chain {
            if !new_chain.contains(n) {
                out.push(EventRecord {
                    node_id: n.0 as u32,
                    event_type: EVT_ROLL_OUT,
                    click_count: 0,
                    pad: [0, 0],
                    touch_id: slot.touch_id,
                    x: slot.last_pos.0,
                    y: slot.last_pos.1,
                });
            }
        }
        for n in &new_chain {
            if !slot.last_hovered_chain.contains(n) {
                out.push(EventRecord {
                    node_id: n.0 as u32,
                    event_type: EVT_ROLL_OVER,
                    click_count: 0,
                    pad: [0, 0],
                    touch_id: slot.touch_id,
                    x: slot.last_pos.0,
                    y: slot.last_pos.1,
                });
            }
        }
        slot.last_hovered_chain = new_chain;
    }

    /// 全局 hovered 合并：清所有 → 所有活跃槽命中链 union（任一指命中元素或祖先 → :hover）。
    fn recompute_hovered(&self, scene: &mut Scene) {
        for n in scene.nodes.iter_mut() {
            n.hovered = false;
        }
        for i in 0..self.slots.len() {
            if i == 0 || self.slots[i].touch_id >= 0 {
                let mut cur = self.slots[i].last_hit;
                while let Some(id) = cur {
                    if id.0 >= scene.nodes.len() {
                        break;
                    }
                    scene.nodes[id.0].hovered = true;
                    cur = scene.nodes[id.0].parent;
                }
            }
        }
    }

    /// 全局 active 合并：清所有 → 所有 is_down 槽的 down_node 命中链 union（基于 down_node，Down 时命中）。
    fn recompute_active(&self, scene: &mut Scene) {
        for n in scene.nodes.iter_mut() {
            n.active = false;
        }
        for slot in &self.slots {
            if slot.is_down {
                let mut cur = slot.down_node;
                while let Some(id) = cur {
                    if id.0 >= scene.nodes.len() {
                        break;
                    }
                    // §4.4：disabled 节点截断 active 链——自身不设 active，其祖先也不（按下 disabled
                    // 子树不应让 disabled 节点或其上层变 active）。逐节点查（不只 down_node）：
                    // hit 落 disabled 节点的非 disabled 子（如 Text 子，坑 29 同款挡命中）时，
                    // 链上遇到 disabled 祖先须截断，而非只查 down_node（原 fix 漏判祖先）。
                    if scene.nodes[id.0].disabled {
                        break;
                    }
                    scene.nodes[id.0].active = true;
                    cur = scene.nodes[id.0].parent;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::{Node, NodeId, NodeKind, Rect, Scene};

    fn one_button_scene() -> Scene {
        // root + button(100x100 at 0,0)
        let mut root = Node::default();
        root.id = NodeId(0);
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut btn = Node::default();
        btn.id = NodeId(1);
        btn.parent = Some(NodeId(0));
        btn.kind = NodeKind::Button;
        btn.layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        root.children = vec![NodeId(1)];
        Scene {
            roots: vec![NodeId(0)],
            nodes: vec![root, btn],
            dynamic_rules: Default::default(),
        }
    }

    /// root + btn(100x100) + btn 的 Text 子(100x20 上半段，挡 btn 上半命中)。
    /// 验 hover 祖先链：hover Text 区（命中 Text）→ Text + btn + root 祖先链都 hovered。
    fn button_with_text_child_scene() -> Scene {
        let mut root = Node::default();
        root.id = NodeId(0);
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut btn = Node::default();
        btn.id = NodeId(1);
        btn.parent = Some(NodeId(0));
        btn.kind = NodeKind::Button;
        btn.layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut txt = Node::default();
        txt.id = NodeId(2);
        txt.parent = Some(NodeId(1));
        txt.kind = NodeKind::Text { content: "btn".into() };
        txt.layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 20.0 }; // btn 上半段，touchable 默认 true 挡命中
        btn.children = vec![NodeId(2)];
        root.children = vec![NodeId(1)];
        Scene {
            roots: vec![NodeId(0)],
            nodes: vec![root, btn, txt],
            dynamic_rules: Default::default(),
        }
    }

    #[test]
    fn hover_text_child_sets_ancestor_btn_hovered() {
        // b 根因回归测：hover btn 的 Text 子区（命中 Text NodeId 2，非 btn NodeId 1）
        // → Text + btn + root 祖先链都 hovered（对齐 fgui rollOverChain + CSS :hover 祖先语义）。
        // 这样 .btn:hover 伪类匹配 btn（即使命中的是 btn 的文字子）。
        let mut s = button_with_text_child_scene();
        let mut ps = PointerState::new();
        // Move 到 Text 区 (10,10)——命中 Text(NodeId 2)，不是 btn(NodeId 1)
        ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Move, x: 10.0, y: 10.0, button: 0, pad: [0, 0], touch_id: -1 }],
        );
        assert!(s.nodes[2].hovered, "Text 子（命中点）hovered");
        assert!(s.nodes[1].hovered, "btn（Text 的祖先）也 hovered——祖先链");
        assert!(s.nodes[0].hovered, "root（btn 的祖先）也 hovered——祖先链");
    }

    #[test]
    fn down_text_child_sets_ancestor_btn_active() {
        // active 祖先链：按下 btn 的 Text 子 → Text + btn 都 active（.btn:active 匹配 btn）
        let mut s = button_with_text_child_scene();
        let mut ps = PointerState::new();
        ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Down, x: 10.0, y: 10.0, button: 0, pad: [0, 0], touch_id: -1 }],
        );
        assert!(s.nodes[2].active, "Text 子（命中点）active");
        assert!(s.nodes[1].active, "btn（Text 祖先）也 active——祖先链");
        // up 后清所有 active
        ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Up, x: 10.0, y: 10.0, button: 0, pad: [0, 0], touch_id: -1 }],
        );
        assert!(!s.nodes[1].active, "up 后 btn active 清零");
        assert!(!s.nodes[2].active, "up 后 Text active 清零");
    }

    #[test]
    fn down_up_same_node_within_threshold_emits_click() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        // Move 到按钮上（触发 RollOver）+ Down + Up（位移 < 10px）
        let evs = vec![
            PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Up, x: 51.0, y: 51.0, button: 0, pad: [0, 0], touch_id: -1 },
        ];
        let out = ps.process(&mut s, &evs);
        let types: Vec<u8> = out.iter().map(|e| e.event_type).collect();
        assert!(types.contains(&EVT_ROLL_OVER), "Move 到按钮 → RollOver");
        assert!(types.contains(&EVT_DOWN));
        assert!(types.contains(&EVT_UP));
        assert!(types.contains(&EVT_CLICK), "同节点位移 <10px → Click");
        assert!(s.nodes[1].active == false, "Up 后 active=false");
        assert!(s.nodes[1].hovered, "hover 保持");
    }

    #[test]
    fn down_up_exceeds_threshold_no_click() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        let evs = vec![
            PointerEvent { kind: PointerKind::Down, x: 10.0, y: 10.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Up, x: 80.0, y: 80.0, button: 0, pad: [0, 0], touch_id: -1 }, // 位移 ~99px
        ];
        let out = ps.process(&mut s, &evs);
        let has_click = out.iter().any(|e| e.event_type == EVT_CLICK);
        assert!(!has_click, "位移超阈值 → 不产 Click");
    }

    #[test]
    fn down_on_disabled_node_no_active_no_click() {
        let mut s = one_button_scene();
        s.nodes[1].disabled = true;
        let mut ps = PointerState::new();
        let evs = vec![
            PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
        ];
        let out = ps.process(&mut s, &evs);
        assert!(!s.nodes[1].active, "disabled 节点 down 不设 active");
        let has_click = out.iter().any(|e| e.event_type == EVT_CLICK);
        assert!(!has_click, "disabled 节点不产 Click");
        let has_down = out.iter().any(|e| e.event_type == EVT_DOWN);
        assert!(!has_down, "disabled 节点不产 Down");
    }

    #[test]
    fn down_held_on_disabled_no_active() {
        // §4.4 回归测：Down 命中 disabled 节点后【按住不松】（无同帧 Up）→
        // disabled 节点及祖先都不应 active。v1c.3 recompute_active 曾漏 disabled 门控
        // （down_node 在 Down handler 无条件赋值，recompute 沿链设 active 不查 disabled）。
        // 注：现有 down_on_disabled_node_no_active_no_click 漏此 case（Down+Up 同 process 调用，
        // recompute 时 is_down 已 false）。
        let mut s = one_button_scene();
        s.nodes[1].disabled = true;
        let mut ps = PointerState::new();
        ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }],
        );
        assert!(!s.nodes[1].active, "按住 disabled btn 不应 active（§4.4 active 抑制）");
        assert!(!s.nodes[0].active, "disabled 祖先 root 也不应 active");
    }

    #[test]
    fn down_held_on_disabled_via_text_child_no_active() {
        // §4.4 回归测（Text 子命中路径）：按下 disabled 按钮的 Text 子（命中 Text，非 btn）→
        // disabled btn 仍不应 active。原 fix 只查 down_node（=Text 子，非 disabled）→ 漏判 disabled 祖先。
        // 坑 29 同款（文字子挡命中）的 active 版：hit 落 disabled 节点的非 disabled 子时，
        // active 链会带上 disabled 祖先——须沿链逐节点查 disabled，不只查 down_node。
        let mut s = button_with_text_child_scene(); // root + btn(1) + Text(2)@(0,0,100,20) 挡 btn 上半
        s.nodes[1].disabled = true;
        let mut ps = PointerState::new();
        ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Down, x: 10.0, y: 10.0, button: 0, pad: [0, 0], touch_id: -1 }],
        );
        // (10,10) 命中 Text 子(2)（Text @0,0,100,20 挡 btn 上半——hover_text_child_sets_ancestor_btn_hovered 已验）
        assert!(!s.nodes[1].active, "按下 disabled btn 的 Text 子 → btn 不应 active（链遍历逐节点查 disabled）");
    }

    #[test]
    fn rollover_emitted_on_enter_rollout_on_leave() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        // Move 到按钮 → RollOver
        let out1 = ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }],
        );
        assert!(out1.iter().any(|e| e.event_type == EVT_ROLL_OVER && e.node_id == 1));
        // Move 移出按钮（150,150 在 root 非 button）→ RollOut(button) + RollOver(root)
        let out2 = ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Move, x: 150.0, y: 150.0, button: 0, pad: [0, 0], touch_id: -1 }],
        );
        assert!(
            out2.iter().any(|e| e.event_type == EVT_ROLL_OUT && e.node_id == 1),
            "移出按钮 → RollOut(button)"
        );
    }

    #[test]
    fn hover_diff_no_move_event_still_runs() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        // 先 Move 到按钮
        ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }],
        );
        assert!(s.nodes[1].hovered);
        // 空事件——hover 应保持（无 RollOut）
        let out = ps.process(&mut s, &[]);
        assert!(
            !out.iter().any(|e| e.event_type == EVT_ROLL_OUT),
            "空事件 hover 保持"
        );
        assert!(s.nodes[1].hovered, "hover 仍 true");
    }

    #[test]
    fn events_preserved_in_generation_order() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        // Move + Down 同帧——Move 的 RollOver 应在 Down 前
        let evs = vec![
            PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
        ];
        let out = ps.process(&mut s, &evs);
        // 找 RollOver 和 Down 的 index
        let ro_idx = out.iter().position(|e| e.event_type == EVT_ROLL_OVER);
        let down_idx = out.iter().position(|e| e.event_type == EVT_DOWN);
        assert!(ro_idx.is_some() && down_idx.is_some());
        assert!(ro_idx.unwrap() < down_idx.unwrap(), "RollOver 在 Down 前（生成序）");
    }

    /// v1c.2: root + parent(100x100) + child(50x50 in parent)。验 hover 祖先链 diff。
    fn nested_scene() -> Scene {
        let mut root = Node::default();
        root.id = NodeId(0);
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut parent = Node::default();
        parent.id = NodeId(1);
        parent.parent = Some(NodeId(0));
        parent.layout_rect = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let mut child = Node::default();
        child.id = NodeId(2);
        child.parent = Some(NodeId(1));
        child.layout_rect = Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 };
        parent.children = vec![NodeId(2)];
        root.children = vec![NodeId(1)];
        Scene { roots: vec![NodeId(0)], nodes: vec![root, parent, child], dynamic_rules: Default::default() }
    }

    #[test]
    fn hover_into_child_no_rollout_parent() {
        // 点 1 回归：hover parent 区(75,75) → 链 [parent,root]；移进 child 区(10,10) → 链 [child,parent,root]。
        // 共同 parent,root → 不产 RollOut(parent)；child 新 → RollOver(child)。
        let mut s = nested_scene();
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 75.0, y: 75.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 10.0, y: 10.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        assert!(!out.iter().any(|e| e.event_type == EVT_ROLL_OUT), "进子 → 不产任何 RollOut");
        assert!(out.iter().any(|e| e.event_type == EVT_ROLL_OVER && e.node_id == 2), "进子 → RollOver(child)");
    }

    #[test]
    fn hover_between_siblings_old_chain_rollout() {
        // 兄弟 A/B：hover A → RollOver(A)+RollOver(root)；移到 B → RollOut(A)+RollOver(B)（root 共同不产）。
        let mut root = Node::default();
        root.id = NodeId(0);
        root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut a = Node::default();
        a.id = NodeId(1); a.parent = Some(NodeId(0));
        a.layout_rect = Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 };
        let mut b = Node::default();
        b.id = NodeId(2); b.parent = Some(NodeId(0));
        b.layout_rect = Rect { x: 100.0, y: 100.0, w: 50.0, h: 50.0 };
        root.children = vec![NodeId(1), NodeId(2)];
        let mut s = Scene { roots: vec![NodeId(0)], nodes: vec![root, a, b], dynamic_rules: Default::default() };
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 25.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 }]);  // 命中 A
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 125.0, y: 125.0, button: 0, pad: [0, 0], touch_id: -1 }]);  // 命中 B
        assert!(out.iter().any(|e| e.event_type == EVT_ROLL_OUT && e.node_id == 1), "移到 B → RollOut(A)");
        assert!(out.iter().any(|e| e.event_type == EVT_ROLL_OVER && e.node_id == 2), "移到 B → RollOver(B)");
        assert!(!out.iter().any(|e| e.node_id == 0), "root 共同祖先 → 不产事件");
    }

    #[test]
    fn hover_chain_idempotent() {
        // 同点 Move 两次 → 第二次无 hover 事件（链不变；Move 仍产——§7.1 恒产，不抑制）。
        let mut s = nested_scene();
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 10.0, y: 10.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 10.0, y: 10.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        assert!(out.iter().all(|e| e.event_type != EVT_ROLL_OVER && e.event_type != EVT_ROLL_OUT),
            "同点 Move → 无 hover 事件（Move 允许，hover diff 幂等）");
    }

    #[test]
    fn hover_out_of_ui_rollout_whole_chain() {
        // hover child → 链 [child,parent,root]；移出根外 → 空链 → 整链 RollOut。
        let mut s = nested_scene();
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 10.0, y: 10.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 300.0, y: 300.0, button: 0, pad: [0, 0], touch_id: -1 }]);  // 根外
        assert!(out.iter().any(|e| e.event_type == EVT_ROLL_OUT && e.node_id == 2), "移出 → RollOut(child)");
        assert!(out.iter().any(|e| e.event_type == EVT_ROLL_OUT && e.node_id == 1), "移出 → RollOut(parent)");
        assert!(!out.iter().any(|e| e.event_type == EVT_ROLL_OVER), "移出 → 无 RollOver");
    }

    // ===== v1c.3 多槽测试 =====

    /// 鼠标 touch_id=-1 进 slots[0]，Down/Up/Click 等价 v1c.2 单指。
    #[test]
    fn mouse_uses_slot0_touch_id_neg1() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        let out = ps.process(&mut s, &[
            PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
        ]);
        assert!(out.iter().any(|e| e.event_type == EVT_DOWN), "鼠标 Down 产");
        assert!(out.iter().any(|e| e.event_type == EVT_CLICK), "鼠标 Click 产");
        assert!(out.iter().all(|e| e.touch_id == -1), "鼠标事件 touch_id=-1");
    }

    /// 两触摸指各自 Down/Up，事件带正确 touch_id。
    #[test]
    fn two_touches_independent_down_up() {
        let mut root = Node::default();
        root.id = NodeId(0); root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut a = Node::default();
        a.id = NodeId(1); a.parent = Some(NodeId(0)); a.layout_rect = Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 };
        let mut b = Node::default();
        b.id = NodeId(2); b.parent = Some(NodeId(0)); b.layout_rect = Rect { x: 100.0, y: 0.0, w: 50.0, h: 50.0 };
        root.children = vec![NodeId(1), NodeId(2)];
        let mut s = Scene { roots: vec![NodeId(0)], nodes: vec![root, a, b], dynamic_rules: Default::default() };
        let mut ps = PointerState::new();
        // touch_id=1 Down 在 A，touch_id=2 Down 在 B（同帧）
        let out = ps.process(&mut s, &[
            PointerEvent { kind: PointerKind::Down, x: 25.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 1 },
            PointerEvent { kind: PointerKind::Down, x: 125.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 2 },
        ]);
        assert!(out.iter().any(|e| e.event_type == EVT_DOWN && e.node_id == 1 && e.touch_id == 1), "touch1 Down@A");
        assert!(out.iter().any(|e| e.event_type == EVT_DOWN && e.node_id == 2 && e.touch_id == 2), "touch2 Down@B");
    }

    /// 5 触摸 Down（slot1-4 满），第 5 指丢弃。
    #[test]
    fn touch_alloc_fourth_dropped() {
        let mut root = Node::default();
        root.id = NodeId(0); root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut s = Scene { roots: vec![NodeId(0)], nodes: vec![root], dynamic_rules: Default::default() };
        let mut ps = PointerState::new();
        // touch_id 1..5 全 Down（4 触摸槽 slot1-4，第 5 指应丢）
        let mut evs = Vec::new();
        for tid in 1..=5i32 {
            evs.push(PointerEvent { kind: PointerKind::Down, x: 0.0, y: 0.0, button: 0, pad: [0, 0], touch_id: tid });
        }
        let out = ps.process(&mut s, &evs);
        let down_count = out.iter().filter(|e| e.event_type == EVT_DOWN).count();
        assert_eq!(down_count, 4, "仅 4 触摸槽，第 5 指 Down 丢弃");
    }

    /// 触摸无 capture Move 不产 Move 事件（hover_diff 仍跑）。
    #[test]
    fn touch_move_no_monitor_no_event() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 51.0, y: 51.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        assert!(!out.iter().any(|e| e.event_type == EVT_MOVE), "无 monitor 触摸 Move 不产 Move 事件");
        assert!(out.iter().all(|e| e.event_type != EVT_MOVE), "无 Move 事件");
    }

    /// 鼠标无 capture Move 不产（v1c.2 行为变化：v1c.2 鼠标 Move 产，v1c.3 不产）。
    #[test]
    fn mouse_move_no_capture_no_event() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 51.0, y: 51.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        assert!(!out.iter().any(|e| e.event_type == EVT_MOVE), "v1c.3 鼠标无 capture Move 不产（对齐 fgui）");
    }

    /// hover 全局合并：两指命中不同元素 → 两元素都 hovered。
    #[test]
    fn hover_global_merge_two_fingers() {
        let mut root = Node::default();
        root.id = NodeId(0); root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut a = Node::default();
        a.id = NodeId(1); a.parent = Some(NodeId(0)); a.layout_rect = Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 };
        let mut b = Node::default();
        b.id = NodeId(2); b.parent = Some(NodeId(0)); b.layout_rect = Rect { x: 100.0, y: 0.0, w: 50.0, h: 50.0 };
        root.children = vec![NodeId(1), NodeId(2)];
        let mut s = Scene { roots: vec![NodeId(0)], nodes: vec![root, a, b], dynamic_rules: Default::default() };
        let mut ps = PointerState::new();
        ps.process(&mut s, &[
            PointerEvent { kind: PointerKind::Move, x: 25.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 1 },  // 命中 A
            PointerEvent { kind: PointerKind::Move, x: 125.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 2 }, // 命中 B
        ]);
        assert!(s.nodes[1].hovered, "A hovered（touch1 命中）");
        assert!(s.nodes[2].hovered, "B hovered（touch2 命中）");
    }

    /// active 全局合并：两指按不同 btn → 都 active；松一指 → 剩余仍 active。
    #[test]
    fn active_global_merge_two_fingers() {
        let mut root = Node::default();
        root.id = NodeId(0); root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut a = Node::default();
        a.id = NodeId(1); a.parent = Some(NodeId(0)); a.kind = NodeKind::Button; a.layout_rect = Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 };
        let mut b = Node::default();
        b.id = NodeId(2); b.parent = Some(NodeId(0)); b.kind = NodeKind::Button; b.layout_rect = Rect { x: 100.0, y: 0.0, w: 50.0, h: 50.0 };
        root.children = vec![NodeId(1), NodeId(2)];
        let mut s = Scene { roots: vec![NodeId(0)], nodes: vec![root, a, b], dynamic_rules: Default::default() };
        let mut ps = PointerState::new();
        ps.process(&mut s, &[
            PointerEvent { kind: PointerKind::Down, x: 25.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 1 },
            PointerEvent { kind: PointerKind::Down, x: 125.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 2 },
        ]);
        assert!(s.nodes[1].active && s.nodes[2].active, "两指都按 → 两 btn active");
        // 松 touch1
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Up, x: 25.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        assert!(!s.nodes[1].active, "松 touch1 → A active 清");
        assert!(s.nodes[2].active, "touch2 仍按 → B 仍 active");
    }

    /// RollOver per-touch：touch1 进 A、touch2 进 B，各自 RollOver 带 touch_id。
    #[test]
    fn rollover_per_touch_independent() {
        let mut root = Node::default();
        root.id = NodeId(0); root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut a = Node::default();
        a.id = NodeId(1); a.parent = Some(NodeId(0)); a.layout_rect = Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 };
        let mut b = Node::default();
        b.id = NodeId(2); b.parent = Some(NodeId(0)); b.layout_rect = Rect { x: 100.0, y: 0.0, w: 50.0, h: 50.0 };
        root.children = vec![NodeId(1), NodeId(2)];
        let mut s = Scene { roots: vec![NodeId(0)], nodes: vec![root, a, b], dynamic_rules: Default::default() };
        let mut ps = PointerState::new();
        let out = ps.process(&mut s, &[
            PointerEvent { kind: PointerKind::Move, x: 25.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 1 },
            PointerEvent { kind: PointerKind::Move, x: 125.0, y: 25.0, button: 0, pad: [0, 0], touch_id: 2 },
        ]);
        assert!(out.iter().any(|e| e.event_type == EVT_ROLL_OVER && e.node_id == 1 && e.touch_id == 1), "touch1 RollOver@A");
        assert!(out.iter().any(|e| e.event_type == EVT_ROLL_OVER && e.node_id == 2 && e.touch_id == 2), "touch2 RollOver@B");
    }

    /// is_pointer_on_ui 任一指命中。
    #[test]
    fn is_pointer_on_ui_any_slot() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        // 鼠标在 UI 外 (150,150 命中 root 非 btn)，触摸在 btn 内
        ps.process(&mut s, &[
            PointerEvent { kind: PointerKind::Move, x: 150.0, y: 150.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 },
        ]);
        assert!(ps.is_pointer_on_ui(&s), "触摸命中 btn → is_pointer_on_ui=true（任一指）");
    }

    // ===== v1c.3 T2: touch_monitors capture 测 =====

    /// Down 后 add_touch_monitor → 后续 Move 产 Move@monitor。
    #[test]
    fn move_with_monitor_dispatches_to_monitor() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        // touch1 Down 在 btn(1)
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        // capture btn（模拟 C# CaptureTouch 后调 add_touch_monitor）
        ps.add_touch_monitor(1, NodeId(1));
        // Move 移出 btn 到 root 区 (150,150)——正常无 monitor 不产 Move，但有 monitor → Move@btn
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 150.0, y: 150.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        assert!(out.iter().any(|e| e.event_type == EVT_MOVE && e.node_id == 1 && e.touch_id == 1),
            "capture 后 Move（即使移出 btn）产 Move@btn");
    }

    /// Up 后 monitor 清空，后续 Move 不产。
    #[test]
    fn capture_clears_on_up() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        ps.add_touch_monitor(1, NodeId(1));
        // Up（清 monitor）
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        // 注意：Up 释放了 slot1（touch_id 重置 -1）。重新 Down 再 Move 验无 monitor
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 2 }]);
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 51.0, y: 51.0, button: 0, pad: [0, 0], touch_id: 2 }]);
        assert!(!out.iter().any(|e| e.event_type == EVT_MOVE), "Up 清 monitor 后 Move 不产");
    }

    /// Up 时 monitor==hit 不重复产 Up。
    #[test]
    fn up_hit_equals_monitor_no_double() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        ps.add_touch_monitor(1, NodeId(1));   // monitor == btn(1)
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        let up_btn = out.iter().filter(|e| e.event_type == EVT_UP && e.node_id == 1).count();
        assert_eq!(up_btn, 1, "monitor==hit → Up@btn 只产一次（去重）");
    }

    /// remove_touch_monitor：加后移除，Move 不再产给该 monitor。
    #[test]
    fn remove_touch_monitor_stops_dispatch() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        ps.add_touch_monitor(1, NodeId(1));
        ps.remove_touch_monitor(NodeId(1));   // 主动释放
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 150.0, y: 150.0, button: 0, pad: [0, 0], touch_id: 1 }]);
        assert!(!out.iter().any(|e| e.event_type == EVT_MOVE), "remove 后 Move 不产给该 monitor");
    }

    // ===== v1c.4 T1: click_test + per-axis 阈值 + down_targets =====

    /// Click 目标 = down_leaf（非当前 hit）。Down@btn 边缘，漂出 btn 到 root（位移≤10），
    /// Up → Click@btn（按下叶），Up 事件@root（当前 hit）。照 fgui ClickTest downTargets[0] 优先。
    #[test]
    fn click_target_is_down_leaf_not_current_hit() {
        let mut s = one_button_scene();   // root(0,0,200,200) + btn(0,0,100,100)
        let mut ps = PointerState::new();
        // Down@(95,50)→btn；Up@(105,50)→root（105>100）。dx=10（mouse 阈值，|10|>10 false→不超）
        let out = ps.process(&mut s, &[
            PointerEvent { kind: PointerKind::Down, x: 95.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Up, x: 105.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
        ]);
        assert!(out.iter().any(|e| e.event_type == EVT_CLICK && e.node_id == 1),
            "Click@btn（down_leaf），即使 Up 时命中已漂移到 root");
        assert!(out.iter().any(|e| e.event_type == EVT_UP && e.node_id == 0),
            "Up@root（当前 hit）");
        assert!(!out.iter().any(|e| e.event_type == EVT_CLICK && e.node_id == 0),
            "不产 Click@root");
    }

    /// per-axis 阈值（非 euclidean）：mouse 对角 (8,8)（euclidean 11.3>10 但 per-axis 8≤10）→ 仍 Click。
    #[test]
    fn per_axis_threshold_mouse_diagonal_clicks() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        let out = ps.process(&mut s, &[
            PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Up, x: 58.0, y: 58.0, button: 0, pad: [0, 0], touch_id: -1 },
        ]);
        assert!(out.iter().any(|e| e.event_type == EVT_CLICK),
            "per-axis (8,8) ≤10 → Click（旧 euclidean 11.3>10 会拒）");
    }

    /// mouse 30px 漂移 → 无 Click（30>10）；touch 30px 漂移 → Click（30<50）。
    #[test]
    fn threshold_mouse_10_rejects_touch_50_allows_30px() {
        // mouse
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        let out_m = ps.process(&mut s, &[
            PointerEvent { kind: PointerKind::Down, x: 10.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
            PointerEvent { kind: PointerKind::Up, x: 40.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 },
        ]);
        assert!(!out_m.iter().any(|e| e.event_type == EVT_CLICK), "mouse 30px >10 → 无 Click");
        // touch
        let mut s2 = one_button_scene();
        let mut ps2 = PointerState::new();
        let out_t = ps2.process(&mut s2, &[
            PointerEvent { kind: PointerKind::Down, x: 10.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 },
            PointerEvent { kind: PointerKind::Up, x: 40.0, y: 50.0, button: 0, pad: [0, 0], touch_id: 1 },
        ]);
        assert!(out_t.iter().any(|e| e.event_type == EVT_CLICK), "touch 30px <50 → Click");
    }

    /// down_leaf 销毁 → 沿当前 hit 祖先兜底。Down@child（scene1），scene2 移除 child，Up@root 区 → Click@root。
    #[test]
    fn down_leaf_destroyed_fallback_to_ancestor() {
        // scene1: root(0,0,200,200) + child(0,0,50,50)
        let mut root = Node::default();
        root.id = NodeId(0); root.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut child = Node::default();
        child.id = NodeId(1); child.parent = Some(NodeId(0));
        child.layout_rect = Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 };
        root.children = vec![NodeId(1)];
        let mut s1 = Scene { roots: vec![NodeId(0)], nodes: vec![root, child], dynamic_rules: Default::default() };
        let mut ps = PointerState::new();
        // Down@(25,25)→child；down_targets=[child(1),root(0)]
        ps.process(&mut s1, &[PointerEvent { kind: PointerKind::Down, x: 25.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        // scene2: 仅 root（child 移除）——NodeId(1) 越界
        let mut root2 = Node::default();
        root2.id = NodeId(0); root2.layout_rect = Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let mut s2 = Scene { roots: vec![NodeId(0)], nodes: vec![root2], dynamic_rules: Default::default() };
        let out = ps.process(&mut s2, &[PointerEvent { kind: PointerKind::Up, x: 25.0, y: 25.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        // click_test：down_targets[0]=NodeId(1) 越界→走祖先；current_hit=root(0) in down_targets → Click@root
        assert!(out.iter().any(|e| e.event_type == EVT_CLICK && e.node_id == 0),
            "down_leaf 销毁 → Click@root（祖先兜底）");
    }

    // ===== v1c.4 T2: 双击 + Move 取消 =====

    /// 双击：两次 Click（time_s 间隔 0.2、同位置、同键）→ 第二次 click_count=2。
    #[test]
    fn double_click_within_window_clickcount_2() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        ps.time_s = 0.0;
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let c1 = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let count1 = c1.iter().find(|e| e.event_type == EVT_CLICK).map(|e| e.click_count).unwrap();
        assert_eq!(count1, 1, "首次 Click count=1");
        ps.time_s = 0.2;
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let c2 = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let count2 = c2.iter().find(|e| e.event_type == EVT_CLICK).map(|e| e.click_count).unwrap();
        assert_eq!(count2, 2, "350ms 内同位同键 → count=2");
    }

    /// 超 350ms → count 重置 1。
    #[test]
    fn double_click_resets_after_window() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        ps.time_s = 0.0;
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        ps.time_s = 0.4;   // >0.35
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let c = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        let count = c.iter().find(|e| e.event_type == EVT_CLICK).map(|e| e.click_count).unwrap();
        assert_eq!(count, 1, "超 350ms → count=1");
    }

    /// 三击循环 1→2→1。
    #[test]
    fn clickcount_cycle_1_2_1() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        let mut counts = Vec::new();
        for i in 0..3 {
            ps.time_s = i as f32 * 0.2;
            ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
            let c = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
            counts.push(c.iter().find(|e| e.event_type == EVT_CLICK).map(|e| e.click_count).unwrap());
        }
        assert_eq!(counts, vec![1, 2, 1], "1→2→1 循环");
    }

    /// Move 位移>50 取消 click：Down→Move 60px→Up → 无 Click。
    #[test]
    fn move_exceeds_50_cancels_click() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Down, x: 10.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        ps.process(&mut s, &[PointerEvent { kind: PointerKind::Move, x: 70.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]); // dx=60>50
        let out = ps.process(&mut s, &[PointerEvent { kind: PointerKind::Up, x: 70.0, y: 50.0, button: 0, pad: [0, 0], touch_id: -1 }]);
        assert!(!out.iter().any(|e| e.event_type == EVT_CLICK), "Move>50 → 取消 click");
        assert!(out.iter().any(|e| e.event_type == EVT_UP), "Up 仍发");
    }
}
