//! 指针输入事件 + 单指针状态机（§10.3）。消费 PointerEvent[] + 命中 → 产 EventRecord[]。
//! v1c.1 单指针。click 阈值 ~10px（§10.3 鼠标）。disabled 节点产 RollOver/Out 但不产 Down/Up/Click。

use crate::hit::hit_test;
use crate::scene::node::{NodeId, Scene};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub x: f32,
    pub y: f32,
    pub button: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Down = 0,
    Up = 1,
    Move = 2,
}

/// 事件输出（FFI 扁平 POD）。event_type: 0=Down,1=Up,2=Move,3=Click,4=RollOver,5=RollOut。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EventRecord {
    pub node_id: u32,
    pub event_type: u8,
    pub x: f32,
    pub y: f32,
}

pub const EVT_DOWN: u8 = 0;
pub const EVT_UP: u8 = 1;
pub const EVT_MOVE: u8 = 2;
pub const EVT_CLICK: u8 = 3;
pub const EVT_ROLL_OVER: u8 = 4;
pub const EVT_ROLL_OUT: u8 = 5;

const CLICK_THRESHOLD_PX: f32 = 10.0;

#[derive(Debug, Clone)]
pub struct PointerState {
    pub last_pos: (f32, f32),
    pub is_down: bool,
    pub down_node: Option<NodeId>,
    pub down_pos: (f32, f32),
    pub last_hovered: Option<NodeId>,
}

impl Default for PointerState {
    fn default() -> Self {
        Self {
            last_pos: (0.0, 0.0),
            is_down: false,
            down_node: None,
            down_pos: (0.0, 0.0),
            last_hovered: None,
        }
    }
}

impl PointerState {
    pub fn new() -> Self {
        Self::default()
    }

    /// hover diff：比较 cur_hover 与 last_hovered，产 RollOut/RollOver 并更新 Node.hovered。
    /// diff 无变化时什么都不做（幂等，每帧可安全重复调）。
    fn hover_diff(&mut self, scene: &mut Scene, out: &mut Vec<EventRecord>) {
        let cur_hover = hit_test(scene, self.last_pos);
        if cur_hover == self.last_hovered {
            return;
        }
        if let Some(old) = self.last_hovered {
            out.push(EventRecord {
                node_id: old.0 as u32,
                event_type: EVT_ROLL_OUT,
                x: self.last_pos.0,
                y: self.last_pos.1,
            });
            scene.nodes[old.0].hovered = false;
        }
        if let Some(new) = cur_hover {
            out.push(EventRecord {
                node_id: new.0 as u32,
                event_type: EVT_ROLL_OVER,
                x: self.last_pos.0,
                y: self.last_pos.1,
            });
            scene.nodes[new.0].hovered = true;
        }
        self.last_hovered = cur_hover;
    }

    /// 消费本帧输入事件 + 命中 → 产 EventRecord 序列。
    /// events 空时仍跑 hover diff（指针位置沿用 last_pos，§6.3）。
    /// hover diff 每个事件后跑（保证同帧多事件的生成序：Move 的 RollOver 在 Down 前）。
    pub fn process(&mut self, scene: &mut Scene, events: &[PointerEvent]) -> Vec<EventRecord> {
        let mut out: Vec<EventRecord> = Vec::new();
        if events.is_empty() {
            // 空事件：指针位置沿用 last_pos，仍跑一次 hover diff（鼠标静止 hover 保持）
            self.hover_diff(scene, &mut out);
            return out;
        }
        for ev in events {
            self.last_pos = (ev.x, ev.y);
            let hit = hit_test(scene, self.last_pos);
            match ev.kind {
                PointerKind::Move => {
                    if let Some(n) = hit {
                        out.push(EventRecord {
                            node_id: n.0 as u32,
                            event_type: EVT_MOVE,
                            x: ev.x,
                            y: ev.y,
                        });
                    }
                }
                PointerKind::Down => {
                    self.is_down = true;
                    self.down_pos = (ev.x, ev.y);
                    self.down_node = hit;
                    if let Some(n) = hit {
                        if !scene.nodes[n.0].disabled {
                            // 命中非 disabled → 设 active + 产 Down
                            scene.nodes[n.0].active = true;
                            out.push(EventRecord {
                                node_id: n.0 as u32,
                                event_type: EVT_DOWN,
                                x: ev.x,
                                y: ev.y,
                            });
                        }
                    }
                }
                PointerKind::Up => {
                    self.is_down = false;
                    if let Some(n) = hit {
                        if !scene.nodes[n.0].disabled {
                            scene.nodes[n.0].active = false;
                            out.push(EventRecord {
                                node_id: n.0 as u32,
                                event_type: EVT_UP,
                                x: ev.x,
                                y: ev.y,
                            });
                            // click 判定：down_node == hit 且位移 <= 阈值
                            if self.down_node == Some(n) {
                                let dx = ev.x - self.down_pos.0;
                                let dy = ev.y - self.down_pos.1;
                                if (dx * dx + dy * dy).sqrt() <= CLICK_THRESHOLD_PX {
                                    out.push(EventRecord {
                                        node_id: n.0 as u32,
                                        event_type: EVT_CLICK,
                                        x: ev.x,
                                        y: ev.y,
                                    });
                                }
                            }
                        }
                    }
                    self.down_node = None;
                }
            }
            // 每个事件后跑 hover diff（生成序：RollOver 在同帧后续事件前；
            // 幂等——hover 不变时无输出）
            self.hover_diff(scene, &mut out);
        }
        out
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

    #[test]
    fn down_up_same_node_within_threshold_emits_click() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        // Move 到按钮上（触发 RollOver）+ Down + Up（位移 < 10px）
        let evs = vec![
            PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0 },
            PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0 },
            PointerEvent { kind: PointerKind::Up, x: 51.0, y: 51.0, button: 0 },
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
            PointerEvent { kind: PointerKind::Down, x: 10.0, y: 10.0, button: 0 },
            PointerEvent { kind: PointerKind::Up, x: 80.0, y: 80.0, button: 0 }, // 位移 ~99px
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
            PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0 },
            PointerEvent { kind: PointerKind::Up, x: 50.0, y: 50.0, button: 0 },
        ];
        let out = ps.process(&mut s, &evs);
        assert!(!s.nodes[1].active, "disabled 节点 down 不设 active");
        let has_click = out.iter().any(|e| e.event_type == EVT_CLICK);
        assert!(!has_click, "disabled 节点不产 Click");
        let has_down = out.iter().any(|e| e.event_type == EVT_DOWN);
        assert!(!has_down, "disabled 节点不产 Down");
    }

    #[test]
    fn rollover_emitted_on_enter_rollout_on_leave() {
        let mut s = one_button_scene();
        let mut ps = PointerState::new();
        // Move 到按钮 → RollOver
        let out1 = ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0 }],
        );
        assert!(out1.iter().any(|e| e.event_type == EVT_ROLL_OVER && e.node_id == 1));
        // Move 移出按钮（150,150 在 root 非 button）→ RollOut(button) + RollOver(root)
        let out2 = ps.process(
            &mut s,
            &[PointerEvent { kind: PointerKind::Move, x: 150.0, y: 150.0, button: 0 }],
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
            &[PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0 }],
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
            PointerEvent { kind: PointerKind::Move, x: 50.0, y: 50.0, button: 0 },
            PointerEvent { kind: PointerKind::Down, x: 50.0, y: 50.0, button: 0 },
        ];
        let out = ps.process(&mut s, &evs);
        // 找 RollOver 和 Down 的 index
        let ro_idx = out.iter().position(|e| e.event_type == EVT_ROLL_OVER);
        let down_idx = out.iter().position(|e| e.event_type == EVT_DOWN);
        assert!(ro_idx.is_some() && down_idx.is_some());
        assert!(ro_idx.unwrap() < down_idx.unwrap(), "RollOver 在 Down 前（生成序）");
    }
}
