# Task 8 Report — 滚轮输入通道（v1d.5）

## Status: DONE（controller 收尾：subagent 中断在 commit 前，改动完整 + 测试绿，由 controller 写报告 + commit）

## Commit
（见本报告下方 controller commit）

## 改动（3 文件，subagent 写入，controller 验证）

- `loomgui_core/src/scroll.rs`：
  - `WheelEvent { x, y, delta_x, delta_y: f32 }`（`#[repr(C)]` + `Default`），16B ABI 断言（`const _: () = { assert!(size_of::<WheelEvent>()==16) }`，坑 34）。
  - `apply_wheel_to_hit(scene, w)`：`hit_test((w.x,w.y))` → 沿 `node.parent` 链找最近 `effective`（x/y 轴）滚动容器 → `scene.scroll.get_mut(id).apply_wheel((delta_x,delta_y))`；无 → 丢弃。独立函数，T10 wire 进 tick。
- `loomgui_core/src/stage.rs`：`Stage.pending_wheel: Vec<WheelEvent>`（init `Vec::new()`）+ `set_wheel_input(&mut self, &[WheelEvent])`（extend，累积式）。**不接 tick**（T10 wire）。
- `loomgui_ffi_c/src/lib.rs`：`loomgui_stage_set_wheel_input(h: *const StageHandle, events: *const WheelEvent, len: usize)`，null-safe（h/events null 或 len 0 → return），照 set_key_input 模式。

## Test summary（controller 跑）
`cargo test --workspace`：core 288 / ffi 38 / pkg 3 全绿，0 failed。`scroll::tests::wheel_steps_and_clamps`（T6 既有）仍绿；apply_wheel_to_hit 独立测绿。

## 自审
- `WheelEvent` `#[repr(C)]` + 16B 断言 ✓（坑 34）。
- FFI null-safe ✓。
- apply_wheel_to_hit 不接 tick（T10 wire）✓ —— 本任务范围正确。
- 不动 blob/pkg version；C# 镜像留 T12（坑 35）✓。
- `pending_wheel` 累积式（多次 set_wheel_input 合并），T10 tick `std::mem::take` 消费 ✓。

## Concerns
无。apply_wheel_to_hit 依赖 hit_test（读 world_transforms）—— T10 tick 里它在 compute_world_transforms 前（用上帧 world_transforms），spec §8.2 认 1 帧差可接受（wheel hit 找容器非精确子命中，stale 不影响）。

## T8 测试补全 fix（reviewer Important）

**问题**：T8 brief Step 4 规定的 3 个测试未写（subagent 中断在测前）。

**补测**：

| 测试 | 位置 | 结果 |
|------|------|------|
| `apply_wheel_to_hit_scrolls_nearest_effective_ancestor` | `loomgui_core/src/scroll.rs` `#[cfg(test)]` | GREEN |
| `set_wheel_input_round_trip` | `loomgui_ffi_c/src/lib.rs` `abi_tests` | GREEN |
| `wheel_event_is_16_bytes` | `loomgui_ffi_c/src/lib.rs` `abi_tests` | GREEN |

**关键发现**：
- `Scene::build` 对 overflow 节点设 `clip_rect=Some(Rect::default())`（(0,0,0,0) 零尺寸），阻死 hit_test 命中整个滚动子树。测试需 hand-fill `clip_rect` 为 `layout_rect` 同尺寸（brief 已点名，但 subagent 未留意）。
- `build_stage` helper 不存在 — 改直接构造 `Stage::new(...)`（既存 abi_tests 已有 `Stage` import 与 font 路径 pattern）。
- `WheelEvent` 16B compile-time 断言已在 scroll.rs:27-29，新增 runtime test 为可见覆盖。

`cargo test --workspace`：core 289 / ffi 40 / pkg 3 / snapshot 3 全部 GREEN（335 测）。
