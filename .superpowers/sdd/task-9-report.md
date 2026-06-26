# Task 9 Report: Scrollbar 合成 RenderNode + Grip 拖拽

**Status**: DONE
**Commit**: `ab55b8b` (main, ponytail)

## Files Changed

| File | Change | Lines |
|------|--------|-------|
| `loomgui_core/src/scroll.rs` | +V_THUMB_FLAG/H_THUMB_FLAG + v_thumb_rect/h_thumb_rect + tests | +136 |
| `loomgui_core/src/render/mod.rs` | +thumb_render_node + 合成 thumb 追加 + tests | +137 |
| `loomgui_core/src/hit.rs` | +hit_scrollbar_grip + hit_test 前置 + tests | +104 |
| `loomgui_core/src/input.rs` | +grip Down/Move/Up + EVT_UP 哨兵守卫 + tests | +134 |

## Implementation Summary

### scroll.rs: Sentinel Flags + Thumb Geometry
- **V_THUMB_FLAG** (`0x4000_0000`) / **H_THUMB_FLAG** (`0x2000_0000`): 合成 RenderNode 高位标记，与容器 NodeId 组合为唯一 sentinel id
- **v_thumb_rect**(scene, id) → `Option<Rect>`: 垂直 thumb 位于右边缘 8px track，尺寸 = `viewport * (viewport/content)`，位置 = `scroll% * (track - thumb)`
- **h_thumb_rect**: 水平轴对称实现，沿底边
- 4 new tests: right-edge position, scroll_pos movement, none-when-no-overlap, bottom-edge horizontal

### render/mod.rs: 合成 Thumb RenderNode
- **thumb_render_node**: 构造半透明灰 `[0.6,0.6,0.6,0.6]` quad，`world_matrix=IDENTITY`（design 绝对坐标），`mask_context=0`（不裁剪），`parent_id=None`
- 在 `build_render_nodes` 末尾、`assign_sort_keys` 之前迭代 scene.nodes，对 effective 容器（`effective(overflow, content, viewport)`）调用 `v/h_thumb_rect` 并 push RenderNode，`sort_key = max(sort_key)+1`
- 2 new tests: effective emits thumb, non-effective no thumb

### hit.rs: hit_scrollbar_grip + hit_test Prepended
- **hit_scrollbar_grip**: 遍历所有容器，依次 check v/h thumb rect → `Some((container_id, axis))`；scrollbar 最上层先到先得
- **hit_test** 顶部前置：命中 grip → 返 `NodeId(container_id | flag)` sentinel
- `point_in_rect` 从 `fn` 改为 `pub(crate)` 供 input.rs 用
- 4 new tests: returns container + axis, none outside thumb, none without scroll, hit_test returns sentinel

### input.rs: Grip Down/Move/Up
- **Down**: 在 scroll 候选查找**之前**调 `hit_scrollbar_grip`，命中 → `grip_dragging=true, scrolling_pane=container, click_cancelled=true, scroll_gesture=axis_bit`，`continue` 跳过 drag/scroll/EVT_DOWN
- **Move**: `scrolling_pane` 有值 + `grip_dragging` → 指针位置驱动 `scroll_pos`（perc = (pos - edge) / (track - min_thumb)，`scroll_pos = perc * overlap`）；非 grip 时仍走原 `drag_follow`
- **Up**: `grip_dragging` 清 → 不启惯性（原 `!slot.grip_dragging` 守卫已到位）；EVT_UP/click 段加 `!grip_dragging` 守卫避免 sentinel NodeId 索引越界
- Up cleanup 加 `slot.grip_dragging = false`
- 4 new tests: grip down sets fields, grip move drives scroll_pos, grip up clears + no inertia, non-thumb area no grip

## RED → GREEN

| Step | Test count before | RED fails | After fix |
|------|-------------------|-----------|-----------|
| scroll.rs helpers | 20 (existing) | +4 new | 24 GREEN |
| render synthetic thumb | 9 (existing) | +2 new | 11 GREEN |
| hit_scrollbar_grip | 8 (existing) | +4 new | 12 GREEN |
| input grip | 73 (existing) | +4 new + 1 sentinel OOB | 77 GREEN |
| **workspace** | **349 total** | **0 failed** | **349 GREEN** |

## Self-Review

1. **Sentinel NodeId 正确性**: `NodeId(pub usize)` 高位 flag 在 render 输出 `node_id: u32 | flag`；hit_test 返 `NodeId((container.0 as u32 | flag) as usize)`；input Down 跳过 hit_test 直接调 `hit_scrollbar_grip` 取 `(container_id, axis)` 避 sentinel；Up 段 `!grip_dragging` 网关防 `scene.nodes[sentinel]` OOB
2. **sort_key**: 手动 `max+1`，不进 `assign_sort_keys` DFS（它扫 scene.nodes，合成节点不在内）
3. **mask_context=0**: thumb 不裁剪，贴 viewport 边缘常显
4. **world_matrix=IDENTITY**: design 绝对坐标 quad，顶点 = thumb rect 的 4 角
5. **effectiveness**: 仅 `effective(overflow, content, viewport)` 且 overlap>0 的容器追加 thumb，`auto` 无溢出无条
6. **零回归**: 既有 335 tests 全绿（render/hit/input/scroll/blobs），非 scroll 容器无 thumb，hit_scrollbar_grip 返 None

## Self-Review: Pitfalls Encountered

- **Sentinel OOB 崩溃**: grip Up 时 `hit_test` 返 sentinel NodeId (`container_id | V_THUMB_FLAG`)，原 EVT_UP 段直接 `scene.nodes[sentinel]` 越界 → 加 `!slot.grip_dragging` 守卫绕开
- **point_in_rect 可见性**: 原 `fn` 私有不对外 → 改为 `pub(crate)` 供 input.rs 的 `hit_scrollbar_grip` 调（同模块 hit.rs 内无问题）
- **base_count 未使用**: brief 里的 `base_count` 仅 doc 意图，移除消 warning

## Concerns

- thumb 仅支持固定 8px track + 20px min thumb（v1 硬编码），未来需要 CSS scrollbar-width/scrollbar-color
- 水平 + 垂直 thumb 同 sort_key（均 `max+1`），若同时存在绘制序由 push 序而定（先 v 后 h）——影响不大（thumb 不重叠）
- grip Move 用固定 dt=0.016 在过程外但与 drag_follow 一致（当前 tick 固定 60fps）

## T9 Critical+Important fix (reviewer)

**Commit**: `f93f282`

### Critical: apply_wheel_to_hit sentinel OOB

**问题**: `apply_wheel_to_hit` 的 while 循环在 `hit_test` 返回 sentinel thumb_id（`container_id | V_THUMB_FLAG` 等）时，直接 `scene.nodes[id.0].parent` 索引越界（sentinel 高位 0x4000_0000 远超 nodes.len()）。

**修法**: while 循环顶部加 sentinel 检查——`id.0 & 0x6000_0000 != 0` 时解码 `id.0 & !0x6000_0000` 得 container_id。thumb 覆盖容器 viewport 边缘，wheel 落 thumb = 落该容器，解码后 container 本身是滚动容器，会 effective 命中 apply_wheel。同时加 `else { break; }` 防御无效 id（sentinel 解码后不应发生）。

### Important: sort_key 时序

**问题**: `build_render_nodes` 在 `assign_sort_keys` **之前**取 `max_sort`（此时全 0 → max=0 → thumb sort_key=1），与 assign_sort_keys 后某真实节点碰撞。

**修法**: thumb 追加逻辑移到 `assign_sort_keys` 调用之后（此时真实节点已分配 sort_key 0..N，max 正确）。`assign_sort_keys` 签名是 `fn(&Scene, &mut [RenderNode]) -> Vec<ClipEntry>`，就地改 nodes。

### 测试

新增 `apply_wheel_to_hit_on_thumb_decodes_sentinel`：构造滚动场景 → 验证 thumb 区域 hit_test 返 sentinel → 调用 apply_wheel_to_hit → 验证不 crash 且容器正确滚动（tweening != 0）。

### 回归

`cargo test --workspace` 350 tests 全绿（+1 新测）。
