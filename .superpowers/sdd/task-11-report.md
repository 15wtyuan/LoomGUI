# Task 11 Report: PlayMode sample + 家里机验收准备

**Status**: completed (pending 家里机验收)
**Commit**: `357d224` feat(v1d.3-T11): PlayMode sample — transform + NativeHost + dump demo
**Date**: 2026-06-25

---

## 1. 创建的文件

### 1.1 `LoomTransformDemo.cs`
- 路径：`loomgui_unity/Assets/LoomGUI/Runtime/LoomTransformDemo.cs`
- 类型：MonoBehaviour，挂在与 LoomStage 同 GameObject
- `[SerializeField] LoomStage _stage`（Inspector 拖入，或 Awake 时 GetComponent 兜底）
- `Start()` 中：
  - `GameObject.CreatePrimitive(PrimitiveType.Cube)` → scale 50,50,50
  - `_stage.BindNativeHost("model-slot", cube)` — 绑定到 HTML `#model-slot`
  - `Debug.Log("[LoomGUI] scene tree:\n" + _stage.DumpScene())` — 输出整树 JSON

### 1.2 sample HTML/CSS（参考文件）
- 路径：`loomgui_unity/Assets/LoomGUI/Samples/v1d3-transform-demo/`
- `index.html`：旋转容器 `.rot` + 剪切 `.skew` + 缩放 `.scale-d` + NativeHost 占位 `#model-slot`
- `style.css`：`.rot{transform:rotate(30deg)}` / `.skew{transform:scale(2,1) rotate(20deg)}` / `.scale-d{transform:scale(1.5)}` / `.inner` 50x50 #ffcc00 子节点

### 1.3 使用方法
在 Unity Inspector 中将 `index.html` 内容粘贴到 LoomStage 的 `_html` 字段，`style.css` 粘贴到 `_css` 字段。或将文本内容通过 Editor 脚本设置。

---

## 2. 家里机 PlayMode 验收清单

家里机 pull `357d224` 后按以下清单逐项验收：

### 准备工作
1. 打开 Unity 项目 `loomgui_unity/`
2. 场景中新建 GameObject，挂 `LoomStage` + `LoomTransformDemo`
3. LoomStage Inspector 中粘贴 sample HTML/CSS（见 Samples/v1d3-transform-demo/）
4. 确保 `_usePackage` = false（inline 加载）
5. Play

### 验收项目

- [ ] **旋转容器** `.rot`：视觉旋转 30°，子 `.inner` 跟随旋转
- [ ] **剪切容器** `.skew`（scale(2,1) rotate(20deg)）：matrix shader 路径渲染正确，无错位
- [ ] **缩放** `.scale-d`：放大 1.5x
- [ ] **命中**：旋转/剪切容器的子节点 `.inner` 点击命中正确（world_to_local 反算）
- [ ] **identity 节点**：现有 UI 节点视觉不变（不回归）
- [ ] **NativeHost cube** 跟随 `#model-slot` 位置 + visible（cube 出现在 model-slot 下方）
- [ ] **DumpScene()** 日志：Console 输出整树 JSON，含 id/classes/layout/world_matrix
- [ ] **Stress**：勾选 `_stress500` → 500 节点无卡顿（≥45fps，FPS 标签可见）

### 验收后
- 若有问题：本机修 → 重编 .dll → push → 家里机复验
- 验收通过：`session-summary`（坑/API 进 knowledge-reference）

---

## 3. Self-Review

| 检查项 | 结果 |
|--------|------|
| C# 语法无错（手检） | ✓ |
| 命名空间一致（LoomGUI） | ✓ |
| BindNativeHost 签名匹配 | ✓（LoomStage.BindNativeHost(string, GameObject)） |
| DumpScene 签名匹配 | ✓（LoomStage.DumpScene() → string） |
| HTML/CSS 与 brief 一致 | ✓ |
| Start() 非 Awake()（LoomStage 已初始化） | ✓ |
| _stage null-safe | ✓ |
| 参考文件与代码文件分离 | ✓（Samples/ 仅参考） |
| 不破坏现有 LoomInteractDemo | ✓（新文件，无修改） |
| Push 成功 | ✓（357d224 on origin/main） |

### 已知注意事项
- `DumpScene()` 在 `Start()` 中调用（tick 前），world_matrix 此时为 identity。完整 world_matrix 需首帧 LateUpdate 后。验收时可在 Console 中确认树结构正确，首帧后在代码中加延迟调用获取非 identity 矩阵。
- 本机无 Unity 编译环境，C# 经手检无语法错，但需家里机 Unity 编译确认。

---

## 4. Final-Review Fix: I1+M1 (non-pure-translation culling + _ObjectMatrix overwrite)

**Commit**: `fix(v1d.3): non-pure-translation culling bounds + MaterialPropertyBlock for _ObjectMatrix`
**Date**: 2026-06-25
**File**: `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs`

### Finding I1 (Important) -- 视锥体剔除错误

非纯平移节点（rotate/scale/skew）GO transform=identity，顶点 box-local (0,0)-(w,h)，shader `_ObjectMatrix` 把渲染移到世界。但 `RecalculateBounds()` 算 box-local bounds（center≈(w/2,h/2)），Unity Renderer 认为网格在原点附近 → 边缘错误剔除（节点靠近屏幕边时闪烁/消失）。

### Finding M1 (Minor) -- 共享 material _ObjectMatrix 覆盖

`MaterialManager.Key` 含 `matrixFlag` 但不含矩阵值。两个非纯平移节点同 (program,texture,maskContext) → 共享 material → `mat.SetMatrix("_ObjectMatrix", m)` 最后写者胜出，除最后一节点外都渲染错矩阵。

### Fix

**M1**: 换 MaterialPropertyBlock（per-renderer，不污染共享 material）:
- `RenderObj` 加 `MaterialPropertyBlock Mpb` 字段（lazy-init `??=`）。
- 非纯平移路径删 `mat.SetMatrix("_ObjectMatrix", m)`，改用 `ro.Mpb.SetMatrix("_ObjectMatrix", m)` + `ro.Mr.SetPropertyBlock(ro.Mpb)`。

**I1**: 平移 Mesh.bounds 到世界位置:
- `RecalculateBounds()` 后（bounds.center 此时 = box-local center），设 `b.center = (Mtx + b.center.x, Mty + b.center.y, 0)` 平移到世界 box center。
- GO transform=identity → `renderer.bounds` = `Mesh.bounds`，culling 正确。
- 旋转/缩放的 AABB 扩展留 v1.x（记注释）。

两处修改：kind=1 (Mesh) 非纯平移分支 + kind=2 (Text) 非纯平移分支。Text 分支额外加每帧 `RecalculateBounds()`（translation 可能不触发 needRebuild）。
