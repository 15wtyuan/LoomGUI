# bug 2 Unity 调试 handoff（按下字消失 + 拖动底消失）

## core 侧已确认正常（dump_interact）

按钮 `:active{transform:scale(0.96)}` 按下，dump 帧 1/2/3：

| 帧 | btn (Container) | Text 子节点 |
|---|---|---|
| frame1 首帧 | Mesh（identity world） | **Text L1**（re-emit） |
| frame2 Down 本帧 | Mesh（active 蓝 #5fb2c4，world 仍 identity——rematch 在 compute_world_transforms 前） | **Unchanged**（Text hash 未变，MirrorPool 保留 frame1 GO） |
| frame3 次帧 | Mesh（world=[0.96,0,0,0.96,2,1]，scale 进 world） | **Text L1**（re-emit，world 同 btn，非纯平移） |

core 没 emit 丢失：frame2 Unchanged 保留上帧 GO，frame3 re-emit Text payload。**字消失在 Unity MirrorPool 侧**。

## 嫌疑点（按可能性排序）

### 嫌疑 1（最可能）：MPB 未覆盖 _ObjectMatrix（CBUFFER uniform）

`LoomGUI-Unlit.shader` `_ObjectMatrix` 在 `CBUFFER(UnityPerMaterial)`（line 43），**不在 Properties block**（ShaderLab 无 Matrix property 类型）。MirrorPool 非 pure 路径用 `Mpb.SetMatrix("_ObjectMatrix", m)` 设。

shader 注释（line 38-40）声称"放 CBUFFER 让 MPB 按 name 覆盖"，但 SRP Batcher 下 MPB 对**非 Properties 的 CBUFFER 字段**覆盖行为微妙——可能 SRP Batcher 用 CBUFFER 默认值（0）而非 MPB 值。

`_ObjectMatrix` 恒 0 → `designWorld = mul(0, v.pos) = 0` → glyph 全塌缩到 design 原点 → root.TransformPoint 后 = 屏幕左上 rootPos → **字"消失"（实际跑到屏幕左上角）**。

**验证**：FrameDebugger 看按下时 Text draw call 的 `_ObjectMatrix` 值（该 = world=[0.96,0,0,0.96,2,1]，不该是 0）。

### 嫌疑 2：Unchanged→re-emit transition 状态残留

frame2 Text Unchanged → MirrorPool 保留 frame1 GO（**pure 路径状态**：localPosition=(Mtx,Mty)、material pure variant 无 OBJECT_MATRIX、无 Mpb）。
frame3 Text re-emit 非 pure → 切非 pure material（OBJECT_MATRIX variant）+ 新建 Mpb + localPosition=zero。

切换时若 material 关键字 / Mpb binding / mesh 顶点（frame1 本地 glyph）与新 _ObjectMatrix 不匹配 → 渲染错。

**验证**：Inspector 暂停 PlayMode，查 frame3 Text GO 的 `MeshRenderer.sharedMaterial` shader keywords 是否含 `OBJECT_MATRIX`、MaterialPropertyBlock 是否非空。

### 嫌疑 3：needRebuild=false 不重建 mesh，color/layout 过时

frame3 Text `needRebuild = fontDirty || LastFontVersion != FontVersion`。font 版本没变 → **不 UploadMesh**（用 frame1 mesh）。若 active 期间 Text 的 color/glyph 该变（理论上 :active 不重继承到 Text 子，但值得确认），mesh 未更新。

**验证**：frame3 Text GO 的 Mesh 顶点颜色是否 = 期望 color。

## 关键对照实验：§3.3 transform

`§3.3 transform` 卡的 `.tr`（`transform:rotate(30deg)/scale(1.4)`）是**静态非 pure** Container + 子 Text（一直非 pure，无 Unchanged→re-emit transition）。

- 若 §3.3 的 `.tr` 蓝底 + 文字**正常显示** → _ObjectMatrix MPB 覆盖工作 → **排除嫌疑 1**，bug 2 是嫌疑 2（transition）。
- 若 §3.3 文字也消失/错位 → 嫌疑 1 坐实（_ObjectMatrix 恒 0）。

## bug 2 B 部分：拖动 1.3 img / 1.4 span 底消失

scroll 时子节点 world 含 scroll offset（**纯平移，pure 路径**，不触发 _ObjectMatrix）。嫌疑是 **clip/culling**：

- `#main-scroll` overflow-y:scroll → clip_rect → CLIPPED shader variant + `_ClipBox`。
- scroll 时子节点移出 viewport 该被 clip（预期），但"该可见的也消失"→ `_ClipBox` 算错或 renderer.bounds culling 误剔除。

**验证**：scroll 到 1.3/1.4 可见位置，FrameDebugger 看这些节点是否有 draw call；查 `_ClipBox` 值 + renderer.bounds。

## 家里机器验收步骤

1. 拉 latest（pkg 已重打 252520 bytes + C# 改动）。**无需重编 .dll**（逗号修复只影响打包期 parse，runtime 用 bincode）。
2. Unity PlayMode 验已修：
   - bug 4 §3：§3.1/3.2/3.3 的 .op/.tr 蓝底显示（逗号修复）
   - bug 4 §2：§2 .flx 内小方块蓝底显示（span→div）
   - bug 1：§1.5 disabled 按钮按下**不变蓝**（保持灰）
   - bug 3：§1.6 NativeHost Cube 不上下颠倒/压扁
3. 若 bug 2 仍在，按上面嫌疑点 FrameDebugger 查（先做 §3.3 对照实验判断嫌疑 1 vs 2）。
