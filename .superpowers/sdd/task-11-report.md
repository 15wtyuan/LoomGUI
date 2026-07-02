# Task 11 Report: showcase driver 重写（layer 骨架 + 导航跳页 + 按页 listener 清 + tips 叠加）

**状态**：complete（本机无 Unity，C# 语法核对 + grep 验证；家里机编译 + PlayMode 验证 = T12）
**日期**：2026-07-02
**spec**：§7.3 导航模型 / §7.4 C# driver 改造 / §7.5 listener 清理约定

---

## 1. 做了什么

重写 `loomgui_unity/Assets/LoomGUI/Runtime/LoomShowcaseDriver.cs`：旧单包切换模型（SubscribeAll + OnDynLoadShowcase 切包重建 scene + scroll nav）→ 新 layer 骨架 + web 式多页导航 + 按页 listener 清 + tips 叠加 + dyn-load-mail 同包 instantiate。

### 1.1 driver 流程（Start → OpenPage → 切页）

```
Start:
 ├─ ConfigureCameraBackground（主相机纯色 = root bg）
 ├─ 建 layer 骨架：
 │   _root = CreateRoot("div", "1080×1920 深蓝底 flex-column")
 │   _uiLayer = CreateNode("div", "flex-grow:1") → AppendChild(_root)
 │   _tipsLayer = CreateNode("div", "column/center/flex-end/padding/pointer-events:none") → AppendChild(_root)
 ├─ LoadPkgBytes("loom_showcase.pkg.bin") 从 StreamingAssets 读字节
 ├─ LoadPackage("showcase", bytes) 进资源池（不建 scene）
 └─ OpenPage("home")

OpenPage(page):
 ├─ if 当前页存在：ClearPageListeners() + RemoveNode(_currentPage)（+ 若 mail 叠加在挂则一并 remove）
 ├─ _currentPage = Instantiate("showcase", page)
 ├─ AppendChild(_uiLayer, _currentPage)
 └─ SubscribePage(page) → 按 page 名 switch 调 SubscribeHome/SubscribeControls/.../SubscribeDynTree
```

### 1.2 layer 骨架

- `_root`（div, 1080×1920, 深蓝底, flex-column）
- `_uiLayer`（div, flex-grow:1）——主界面层，各页 instantiate 后挂此
- `_tipsLayer`（div, column/center/flex-end/padding:40px/pointer-events:none）——tips 层，挂 _root 末尾（在 _uiLayer 之上）

tips_layer 用 `pointer-events:none` 不挡点击；tips_toast instantiate 后挂此层 + Coroutine 2s 后 RemoveNode。

### 1.3 导航跳页（模型 2 web 式）

home 的 nav-* 按钮 → `OpenPage(targetPage)`：清当前页 listener + RemoveNode(当前页) + Instantiate(目标页) + 挂 ui_layer + SubscribePage(目标页)。

各 page 的 `back-home` 按钮 → `OpenPage("home")`（同一机制，home 也是页）。

### 1.4 按页 listener 清（§7.5）

driver 维护 `Dictionary<uint, List<(EventType, EventCallback)>> _pageListeners`（当前页注册的 listener）：

- `AddPageListener(node, type, cb)`：调 `EventHandler.AddListener` + 记进 `_pageListeners[node]` 列表
- `ClearPageListeners()`：遍历 `_pageListeners` 逐个 `EventHandler.RemoveListener`，最后 `_pageListeners.Clear()`
- 调用时机：`OpenPage` 里 RemoveNode(_currentPage) 之前调 ClearPageListeners（RemoveNode 后旧 NodeId 失效，listener 成悬空条目——须先清）

**不用 `EventHandler.Clear()` 粗清**（spec §7.5：太粗，清所有页/全局 listener）。LoomEventHandler.cs 无需改动——AddListener/RemoveListener 已是细粒度 API，注册表逻辑全在 driver 侧。

### 1.5 tips 叠加

home 的 `nav-tips-demo` 按钮 → `ShowTips()`：
- Instantiate("showcase", "tips_toast") → AppendChild(_tipsLayer)
- StartCoroutine(RemoveAfter(toast, 2.0f))：WaitForSeconds(2) 后 RemoveNode(toast)
- `_tipsRoutine` 守卫防重复触发叠多个 toast

tips 挂 _tipsLayer（非 _uiLayer），故切页时 tips 不被摘（自然叠加在所有页之上）。

### 1.6 dyn-load-mail 适配（T10 交接）

page_dyntree 的两个按钮：
- `dyn-load-mail` → `OnDynLoadMail`：Instantiate("showcase","mail") → AppendChild(_uiLayer)（叠加在 page_dyntree 之上，**非切包**——mail 是 showcase 包内组件）。记 `_mailOverlay` 防重复挂。UpdateDynLoadStatus 改 dyn-load-status 文本。
- `dyn-load-showcase` → `OnDynLoadShowcase`：RemoveNode(_mailOverlay)（摘邮件叠加）。清 `_mailOverlay`。

mail 是 showcase 包内组件（T10 合进），不需 LoadPackage 切包。OpenPage 切走 page_dyntree 时也会顺手 remove mail（防 mail 残留到下页）。

---

## 2. 各页订阅映射（SubscribePage → SubscribeXxx）

| page | Subscribe 方法 | 订阅的语义 id | 复用旧 driver 逻辑 |
|---|---|---|---|
| home | SubscribeHome | nav-controls/nav-text/nav-image/nav-scroll/nav-tween/nav-interact/nav-dyntree → OpenPage(目标页)；nav-tips-demo → ShowTips | 新建（旧 driver 是 scroll nav） |
| page_controls | SubscribeControls | back-home；btn-demo-disabled → SetNodeDisabled；model-slot → BindNativeHost | 旧 SubscribeAll 的 disabled + NativeHost 部分 |
| page_text | SubscribeText | back-home（纯展示） | 新建 |
| page_image | SubscribeImage | back-home（纯展示） | 新建 |
| page_scroll | SubscribeScroll | back-home（page-scroll 自带滚动行为） | 新建 |
| page_tween | SubscribeTween | back-home；tween-play/ease-play/delay-play/complete-play/kill-btn/clear-btn + t-opacity TweenComplete + kill-target 旋转 | 旧 SubscribeTweenDemos（完整复用 OnTweenPlay/OnEasePlay/OnDelayPlay/OnCompletePlay/OnTweenCompleteTag/OnKill/OnClear） |
| page_interact | SubscribeInteract | back-home；hit-click/hit-hover/hit-drag/hit-longpress/hit-key + hit-disabled + route-outer/inner/pe | 旧 SubscribeLampEvents（完整复用 OnClickHit/OnHoverHit/.../OnRouteOuter/Inner/Pe + LightLamp） |
| page_dyntree | SubscribeDynTree | back-home；dyn-add/add20/del/clear/style + dyn-anchor + dyn-load-mail/showcase | 旧 SubscribeDynamicTree（复用 CreateDynPanel/OnDynAdd/.../OnDynStyle；dyn-load-* 改 instantiate/remove） |

所有订阅走 `AddPageListener`（记进注册表），不再直接 `EventHandler.AddListener`。

### 2.1 删的旧逻辑

- `SubscribeAll`（单页全订阅）→ 拆成 SubscribePage + 各 SubscribeXxx
- `OnNavClick` + `_navNodes` + `_scrollNode` + `_sectionY`（scroll nav 跳区）→ 改 OpenPage 跳页
- `StaggeredEntrance`（启动错峰入场，基于 sec-1..8）→ 删（各页独立 instantiate，无 sec 序列）
- `OnDynLoadShowcase` 切包逻辑 → 改 remove mail
- `OnDynLoadMail` TODO stub → 改 instantiate mail
- `LoadPackageFile`/`_dynLoadCurrent`/`ShowcasePkg`/`MailPkg` 常量（切包用）→ 改 `LoadPkgBytes` + `Instantiate`
- `EventHandler.Clear()` 粗清 → 按页清（ClearPageListeners）

---

## 3. 文件变更

- Modify: `loomgui_unity/Assets/LoomGUI/Runtime/LoomShowcaseDriver.cs`（重写，342→532 行）
- LoomEventHandler.cs：**未改**（AddListener/RemoveListener 已是细粒度 API，注册表逻辑在 driver 侧）

---

## 4. 自查

- [x] layer 骨架：create_root + ui_layer + tips_layer（tips 在上，pointer-events:none）
- [x] 导航模型 2（web 式）：home 也是页，点 nav remove 当前页 + instantiate 目标页占满 ui_layer
- [x] 按页 listener 清（§7.5）：driver 维护 _pageListeners 注册表，切页前 ClearPageListeners 批量 RemoveListener
- [x] tips 叠加：instantiate tips_toast 挂 tips_layer + Coroutine 2s 定时 remove
- [x] dyn-load-mail：instantiate mail 组件（非切包），dyn-load-showcase：remove mail
- [x] 各页订阅拆 SubscribeHome/SubscribeControls/.../SubscribeDynTree，instantiate 后按页调
- [x] 零回归：灯阵（page_interact）/tween（page_tween）/动态树（page_dyntree）回调逻辑完整复用旧 driver
- [x] grep 验证：无 LoadPackageFile/切包/EventHandler.Clear 粗清/onDynLoadShowcase 切包/_dynLoadCurrent 残留
- [x] C# 语法核对：using System.Collections.Generic + UnityEngine；EventType/EventCallback/EventContext 在 LoomGUI namespace 直接可见；`_ =>` lambda 与 LoomEventHandlerTests.cs:135 一致（C# 9 discard param，Unity 2021+ 默认）
- [x] 本机无 Unity 工具链，不编译；家里机 T12 编译验证

---

## 5. 关切（家里机 compile/PlayMode 可能 catch）

1. **tips_layer 挤压 ui_layer**：tips_layer 无子时 padding:40px 占 ~80px 高，ui_layer 被 flex-grow:1 挤缩 ~80px。tips_toast 挂载后 tips_layer 更高。v1 无 position:absolute（spec §1 非目标），叠加定位是 v1.4-b 事。PlayMode 看 tips 是否视觉上"在上"——若挤压明显，T12 可调 tips_layer 高度 0 + overflow:visible 让 toast 溢出渲染（不改逻辑）。
2. **mail 叠加定位**：mail 组件（600×800）挂 ui_layer（column），会排在 page_dyntree 下方而非视觉覆盖。spec §7.3 说 mail 挂 ui_layer（或专用 overlay layer）——本 task 按 brief 挂 ui_layer 演示 instantiate 机制。视觉覆盖同上 v1.4-b 事。
3. **场景 SampleScene 旧字段**：driver 的 _sectionY 已删，scene YAML 仍存 _sectionY 数据（281-319 行）→ Unity 反序列化 warning "field not found"（不崩，不阻断编译）。T12 场景清理可删。
4. **StreamingAssets 路径**：LoadPkgBytes 用 File.ReadAllBytes(Application.streamingAssetsPath + "loom_showcase.pkg.bin")。editor/standalone OK；Android 是 jar:file:// 需 UnityWebRequest——本 showcase 只跑 editor/standalone，T12 验。
5. **Coroutine 跨 scene**：RemoveAfter 协程持 node 句柄；若 scene 卸载 MonoBehaviour disable 会自动停协程；LoomStage.OnDestroy 后 _stage=null，RemoveNode 内 `if (_stage==null) return 0` 防崩。
6. **id 多实例**：各 page 的 back-home/page-scroll 等 id 在包内唯一，但 instantiate 多实例会重复（spec §8.1 约定 find_node_by_id 返首个）。本 driver 每页只 instantiate 一次 + 切页前 remove，无多实例冲突。

---

## 6. 与 spec §7.3-7.5 对照

| spec 要求 | 实现 |
|---|---|
| §7.3 create_root + ui_layer + tips_layer | Start 建 _root + _uiLayer(flex-grow:1) + _tipsLayer(pointer-events:none) |
| §7.3 load_package + instantiate home | LoadPackage("showcase", bytes) + OpenPage("home") |
| §7.3 home nav → remove current + instantiate target | OpenPage: ClearPageListeners + RemoveNode + Instantiate + AppendChild + SubscribePage |
| §7.3 页内返回回 home | SubscribeBackHome: back-home → OpenPage("home") |
| §7.3 tips → instantiate tips_toast 挂 tips_layer + 定时 remove | ShowTips: Instantiate + AppendChild(_tipsLayer) + Coroutine 2s RemoveNode |
| §7.4 不再 LoadPackageFile 切包 | 删 LoadPackageFile/OnDynLoadShowcase 切包；dyn-load-mail 改 instantiate |
| §7.4 各页订阅拆 SubscribeHome/.../SubscribeDynTree | SubscribePage switch 8 页 |
| §7.5 driver 维护 listener 注册表 | _pageListeners Dictionary<uint, List<(EventType, EventCallback)>> |
| §7.5 切页前批量 RemoveListener | ClearPageListeners 遍历 RemoveListener |
| §7.5 不用 EventHandler.Clear 粗清 | ClearPageListeners 细粒度按页清（grep 确认 Clear 只在注释） |
