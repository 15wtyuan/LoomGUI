# Changelog

All notable changes to `com.yio.unity` will be documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **图集页纹理逐出（#62）**：`SpriteResolver` 页缓存从「懒加载永不释放」升级为
  expireAfterAccess 逐出——页从最后一次被画起计时，闲置超 `PageEvictionGraceSeconds`
  （默认 10s，≤0 禁用）即销毁纹理、再次使用时现场重载。逐出证据两路：变更帧 `GetSprite`
  盖章 + `MirrorPool.Sync` 每帧对 active 镜像对象的绑定页盖章（Skip 行不进 lean 段、变更
  帧零 GetSprite——缺镜像侧续命则静态页的图集页会被误逐、材质引用已销毁纹理）。逐出按
  各页最后使用独立倒计时、错峰死亡，无批量回收峰值；字体页独立字典结构豁免；仅 PlayMode
  生效。观察面：`UnityYioBackend.Sprites`（`PagesAlive` / `PagesEvictedTotal`）。
  showcase 新增 `texture-lab` 页：三组图标各挂独立图集（evict-a/b/c），开关+读数摆台
  验收闲置逐出 / 独立倒计时 / 重载无缝。
- **图标署名（CC BY 3.0）**：showcase 图标（含存量与新增 evict-a/b/c 测试图集）来自
  [game-icons.net](https://game-icons.net/)，作者 Lorc、Delapouite & contributors，
  许可 Creative Commons Attribution 3.0——存量图标自此补记署名。

### Changed
- **box-sizing 契约定版 content-box（#116，breaking）**：尺寸语义 = CSS 规范初始值——
  `padding adds to the set width/height`（`width:420px + padding:22px` 渲染 464 外框），
  `box-sizing` 声明依旧围栏硬拒、报错文案改为 content-box 减法指引。五层收敛：core 钉
  ContentBox 不动；fence 报错文案改写（曾与契约矛盾八个月）；preview base.css / rect-diff
  reset.css 去除 `*{box-sizing:border-box}` 重置（预览对拍器从此说真话），并中立化 Chrome
  UA 对 form 控件自钉的 border-box；showcase 17 页 55 条「显式尺寸+padding」规则手算折算。
  **根节点语义**：solve 期把 Stage 钉入的 root_size 以 `box_sizing=BorderBox` 覆写根 taffy
  style（浏览器 ICB 同构）——root 自身 padding 内缩而非外溢出画布；全屏页惯用法 =
  `width:100vw; height:100vh` + 零 padding，内缩交给子级（home 页已按此改写）。
- **围栏选择器放行子代组合器 `>`（#114）**：复合控件嵌套态样式作用域——状态属性在控件
  宿主（`[aria-selected]` 等）、视觉落在直接子 wrapper 时，后代选择器会连带命中嵌套同构
  后代（选「材料」点亮嵌套「金属」行）；`>` 限直接父，`+`/`~` 继续拒。core 匹配器
  Child 语义既有（scope 批已实现，本批补单测），打包期 control-css 命中校验同步 Child。
  fence.md / css-reference.md 组合器小节同步。
- **项目更名 Ikat → Yio（breaking）**：品牌、crate 与二进制名（`ikat_core`→`yio_core` 等、
  `ikat`→`yio`、`ikat_gui.exe`→`yio_gui.exe`）、DLL 与 FFI 符号前缀（`ikat_ffi_c.dll`→
  `yio_ffi_c.dll`、`ikat_*`→`yio_*`）、C# 命名空间与程序集（`Ikat.*`→`Yio.*`）、Unity 包 id
  （`com.ikat.unity`→`com.yio.unity`）、工作区配置文件名（`.ikat/`→`.yio/`、
  `ikat.workspace.json`→`yio.workspace.json`、`ikat.runtime.json`→`yio.runtime.json`）、
  DSL token（`ikat-hook`→`yio-hook`、`ikat-preview`→`yio-preview`）全量更名。磁盘格式身份
  同步换血：pkg 魔数 `LPKG`→`YPKG`、frame blob 魔数 `LOOM`→`YIO1`——旧 pkg.bin 全部失效，
  须用新 `yio.exe` 重打；旧配置文件名与 DSL token 不再识别；消费侧 `Packages/manifest.json`
  的包 id 引用须同步改。

### Added
- **组件文件按 stem 可实例化**：`components/` 目录文件直收进 pkg 组件映射，
  运行时 `Instantiate("my-widget")` / `GetTemplate` 按 stem 克隆（fgui 组件一等公民
  同构；api-reference 既有承诺的兑现）。独立实例化走组件语义——`<slot>` 无 light 子
  可投影、全走 fallback 原位渲染，文件内嵌套 hyphen 标签照常经注册表展开。
  页面与组件撞名打包期 fail loud。`yio list/show` 的页面计数不含组件条目。
- **`::part()` 跨组件精确样式通道（#57）**：页面规则穿组件内容墙命中组件内部节点——
  `prefix::part(name)` 中 compound 前缀匹配组件 host、`::part(name)` 匹配展开子树内带
  `part="name"` 属性的目标节点（`part` 全局属性入围栏 + attrs β 仓持久化）。一层不递归
  （嵌套组件内部不可达）；specificity 按 web（`.card::part(title)` = (0,2,1)）；必须位于
  最后一个 compound 结尾。运行时 `StyleSheet.Add` 同语法可用。pkg v57（与 #52 shape mask 并行撞号合并批——两批各自中间态 v56 互斥，合并升 57）。preview server 把 `::part(n)` 平铺重写为 `[part="n"]` 后代选择器
  （浏览器平铺 DOM 无 shadow，保真不缺样式）。
- **组件类绑定 RegisterComponent（#20）**：`UIContext.RegisterComponent(tag, factory)`
  把 custom tag 绑定到 C# 派生类（fgui extensionCreator 等价）——显式工厂委托构造
  （IL2CPP/AOT 零反射），派生 `CustomElement` 子类（`protected internal` 基类构造）。
  构造路径全覆盖（instantiate eager / 懒物化 / 事件预物化），`OnConnected` 在派生
  ctor 完成后回调；`OnDisconnected` 双路径（用户 Dispose 同步 / Rust 侧删除经
  `PumpRemovedNodes` 帧泵——list 槽位换绑、外部 remove_node 均覆盖），回调后 wrapper
  标 `IsDisposed`。重挂 = 新实例新 OnConnected。晚注册只影响未来构造；重复注册/空
  tag/null 工厂抛 `UIContractException`。附带：节点死亡通知队列（core
  `Scene::free_node_slot` 单一漏斗 + FFI `yio_stage_drain_removed_nodes`），Rust 侧
  死亡的滞留 wrapper（含非组件）随泵 evict——死亡显式化，不再死 id 静默打 FFI。
- **shape mask（#52）**：`clip-path` 非矩形几何遮罩——fence 子集
  `circle(<length|%> [at <pos>])` / `polygon(<x> <y>, ...)`（3..=16 点，硬边）。
  声明即 clipper（裁自身 + 子树，与 overflow 独立），命中测试同形（被裁区域
  点击穿透，浏览器行为）；circle 百分比半径按 CSS 对角线归一语义（正方形
  `circle(50%)` = 内切圆）。运行时 CSS 通道接通（`StyleSheet.Add` 类规则 /
  `var()` 代换）。构建期硬拒组合：`overflow:scroll/auto` + `clip-path`、
  裁剪链深 > 4。
### Changed
- **尺寸语义修正为 CSS 初始值 content-box**：声明 `width`/`height` 现为内容尺寸，
  padding/border 加在外侧（浏览器 UA 缺省语义；`ResolvedStyle` 的 taffy
  `box_sizing` 显式钉 `ContentBox`）。此前 taffy 0.14 默认 BorderBox（#82 升级时
  未显式设置、静默翻转）导致「定宽 + padding」元素运行时外框比浏览器预览窄
  padding 两圈——引擎与文档（css-reference "padding adds to the set width/height"）
  矛盾，#52 shape-mask 验收实锤。现有页面凡定宽+padding 元素外框会变大一圈
  （= 与预览一致方向）。pkg v57 重打。
- **裁剪管线泛化为多 entry clip 栈（#52，web 语义对齐）**：嵌套裁剪不再坍缩成
  单矩形，每 context 携带整条祖先链（rect/圆角/circle/polygon 逐条交集）——
  圆形头像放进滚动列表等组合正确。**祖先圆角现在裁后代角**（此前不传播，已对齐
  浏览器）。**裁剪形随裁剪器自身 transform 旋转**（此前不旋转的预览分歧已移除）。
  命中测试感知圆角与形状（此前纯矩形判定）。渲染 blob clip 表换 92B 多 entry
  布局（pkg v57——与 #57 ::part 并行撞号，合并批共升）。
- **嵌套滚动条 thumb 随外层滚动（#52 验收轮修复）**：合成 thumb 的 design
  rect 此前不含祖先滚动偏移——滚动容器嵌在另一个滚动容器里（首个形态：
  shape-mask 页 D 区）时 thumb 钉死在静态位置。现沿祖先链累计 scroll_pos
  补偿（与后代 world_matrix 的 T(-scroll) 注入同语义），嵌套两层级单测覆盖。
- **运行时 CSS 与 custom props（#11）**：`UIContext.StyleSheet.Add/Clear` 接通——
  运行时注入 CSS 规则（围栏选择器+声明子集，含 `--*` 自定义属性与 `var()` 值；
  at-rule 一律拒），解析失败抛 `UIStyleException` 带 `Line`/`Column`。注入规则
  与模板 CSS 同 cascade 优先级（同 specificity 后 Add 赢）、全局跨作用域命中
  （打包期组件内容墙不约束运行时注入），下一帧生效；`Add` 返回 `IDisposable`
  句柄、Dispose 撤销。`Style.SetVar` ×4（`Length`/`YioColor`/`float`/`string`）
  / `RemoveVar` 接通——CSS 自定义属性最高优先级层，`RemoveVar` 回落 CSS 声明值。
  CSS 侧（打包期）同步开 `--x: val` 声明（样式表 + 行内 style）与 `var(--x[, fallback])`
  消费（任意属性值位、嵌套引用、继承跨组件边界）；引用环打包期 warning
  （`FenceCustomPropCycle`）、运行时 invalid 回退 + warn-once（不抛异常）。
- **ListView 多模板（#12）**：`TemplateSelector` 从纯 C# 缓存升级为真正参与克隆——
  list 下可声明多个 `<template id>` 蓝图，`GetTemplate` 取出塞进 selector lambda
  按 index 逐项分派。严格派语义：selector 设了即全权（返 null / 裸包组件模板抛
  `UIContractException`）；与 `ItemTemplate` 同设 selector 赢（默认蓝图语义保留）。
  core 侧按模板分池克隆复用（slot 永驻不变量保持），spacer 估高按蓝图均值分化，
  `Notify*` 后 C# 侧重推受影响区间。多模板未给选择（无 ItemTemplate/selector）时
  首次 `ItemCount` 抛 `UIContractException`。
### Fixed
- **树叶子条目点击激活父分组**：leaf treeitem 原不持控件态，指针激活的目标解析
  （find_control_at 按控件态上溯）会跳过叶子落到父 branch——点叶子 = 选中父 +
  误折叠。全部 treeitem 持 `TreeItem` 态（leaf `expanded` 恒 false）；`expanded`
  读写（FFI `get/set_treeitem_expanded` → -1）与 `aria-expanded` 合成仍
  branch-only，`IsBranch`/`Expanded` 的 C# 契约不变。web 预览模拟器行为不变。
- **`TextNode.Text` / `Image.Src` getter 读 pkg 烙入值恒空串**：读侧原为 C# 镜像（仅
  set 路径更新），`Instantiate` 出来的 HTML 文本/图片 src 从不过 C# setter → getter
  对合法初值返 ""。getter 改直读 core 真值（新 FFI `yio_stage_get_node_text` /
  `yio_stage_get_src`），`Container.TextContent` 读侧随之修正。
- **enter 前设 `ItemTemplate` 被静默丢弃**：旧路径 `yio_list_set_template` 在
  未进数据驱动模式时返 -1 且 C# 不查 rc——先设 `ItemTemplate` 再设 `ItemCount`
  的常规顺序会丢模板（列表默默用 HTML 备用蓝图）。现 enter 前的模板设定缓冲到
  enter 时消费；enter 后设 ItemTemplate 遇陈旧源（节点已删）抛
  `UIContractException`（旧路径静默无效果）。
- **Tree 复合控件（#8）**：`<div role="tree">` / `<div role="treeitem">`
  WAI-ARIA 标准树。条目直接嵌套（treeitem 内嵌 treeitem，无 group 包装层），
  任意深度；branch（有直接 treeitem 子）可展开/折叠（`aria-expanded="true"`
  烘焙初值、缺省折叠），leaf 纯文本。单选树（焦点移动即选中，APG 单选模型），
  初始选中 = 首个 `aria-selected="true"` 条目（无则首项）。键盘 = APG Tree
  View 核心档：↑↓ 可见项间移动、→ 展开/进首子项、← 折叠/回父项、Home/End
  首末可见项、Enter/Space 激活（选中 + branch 折叠互切）；方向键长按连发
  （key repeat）。样式钩子（synth aria，作者 HTML 不写）：`[aria-selected="true"]`、
  `[aria-expanded="true"]`、`[aria-level="N"]`（层级缩进用，顶层=1）。运行时
  API：`Tree.SelectedItem` / `ExpandAll()` / `CollapseAll()` / `SelectionChanged`、
  `TreeItem.IsBranch` / `Expanded` / `Selected` / `Level` / `Select()` /
  `ExpandedChanged`。围栏结构门：tree 缺直接 treeitem 子报
  `FenceMissingControlChild`。branch 的 label 走子元素（wrapper div）——branch
  宿主嵌套条目为 block 子，纯文本 label 触发 `FenceMixedInlineBlock`；leaf
  纯文本合法。
- **attrs β 通用属性仓（#8/#22）**：per-Node 白名单 HTML 属性持久化
  （现 `aria-controls` / `aria-labelledby`），pkg v53 起退役 aria_controls
  专列、单一路径入仓；`aria-labelledby` 端到端打通（fence 校验 → pkg →
  RoleInfo），运行时关联机制 `Scene::attr_idrefs`（逐 id 本作用域解析，
  与 aria-controls 同路）。
- **拖拽使能双通道（#75）**：HTML `draggable="true|false"`（全局属性，缺省
  false；浏览器枚举值 `auto` 在自绘引擎无对应物，围栏报错）+ 运行时
  `Node.Draggable`。使能节点参与 drag_target 候选，pointer-down 过阈值后
  DragStart/DragMove/DragEnd 事件链启动（此前事件链全通但产品面无任何
  使能入口）。
- **TabList 手动激活模型（#13）**：`data-activation="manual"`（tablist 上
  值域 manual|automatic，缺省 automatic）+ 运行时 `TabList.Activation`。
  manual = 方向键只移焦点（roving tabindex，焦点移动发 FocusIn/FocusOut，
  `:focus` 样式跟随）、Enter/Space 才提交选中；automatic = 方向键即时选中
  **且焦点跟随**（对齐 WAI-ARIA，修正旧实现不移焦点的偏差）。roving 步进
  做成通用积木（clamp 不 wrap），后续 Tree 复用。
- **键盘按住自动重复（#76）**：长按 Backspace/Delete/方向键等连续生效
  （连删/连续导航）。Unity 两代输入系统都不发 OS 键盘重复事件，collector
  按 OS 节律合成重复 keydown（0.5s 初始延迟 + 0.03s 间隔，固定常量）；
  最后按下的键优先、keyup 即停、失焦清状态。仅重发 keydown 通道——
  可打印字符不重复插字（KeyCode→char 映射键盘布局相关，不做）。
- **多 Stage 隔离：共享相机 + 排序基址 + 输入独占路由（#109）**：同场景多
  Driver 并存不再互相打爆——per-Scene 引用计数共享 `YioUICamera`（按名
  认领存量相机先于新建，编辑器重编译幸存相机不再积累重复）；各 Stage
  sortingOrder 基址 = 层序 × 8192（16 位预算 4 档）；多 Driver 时输入按
  层序顶→底探测首个 Pick 命中者独占本帧全部输入（单 Driver 零开销直通）。
- **世界锚点（投影路世界 UI，#109）**：`YioStageDriver.SetWorldAnchor /
  ClearWorldAnchor`——每帧把 3D 世界点经相机投影成设计坐标写
  `node.Transform.Position`（跳字/血条类 HUD 跟随 3D 实体）；出屏/相机
  背后自动隐藏（渲染层开关，与 `display:none` 正交；**继承语义**：隐藏
  祖先 = 整子树隐藏，后端保留镜像对象仅 SetActive(false)）。
- **world-space 子树挂载（#109）**：`YioStageDriver.BindWorldMount /
  UnbindWorldMount`——整棵 UI 子树挂到业务 3D 变换下渲染（行顶点 re-base
  到挂载根局部系 + 按槽位路由 SetParent，容器层随业务 → 场景相机渲染 +
  ZTest LEqual 吃 3D 深度遮挡）；布局/命中仍在屏幕系。v1 约束：挂载根须
  声明 z-index 成 stacking context；挂载内禁 dropdown / 滚动容器 / 外阴
  影根 / overflow clip。
- **共享资源宿主（#109）**：多 Stage 共享字体注册/字形图集/图片尺寸表
  （`YioResourceHost`），字体注册一次全 Stage 复用（无 id churn / 图集
  重光栅）；单 Stage 行为不变。

### Changed
- **frame blob v15 列级增量（#109）**：未变更行出 SOA 进段末 16B/条 Skip
  段；胖参数块（color_matrix/effect/shadow/gradient）挪 fat arena（全零
  不写）。全 Skip 帧带宽 132+16n 字节（v14 为 128+512n）。
- **增量渲染构建（#109）**：输入指纹命中的节点整段复用上帧渲染产物——
  ~2400 节点稳态帧构建耗时约减半；500 血条压测稳态 ~9ms（对照全量重建
  ~16.9ms）。
- **fit 模式 safe-area 基座反转：贴物理边 + CSS 级避让（#110）**：`fit-width` /
  `fit-height` 的画布不再从 `Screen.safeArea` 起算（旧行为在异形屏留出一条
  不被 UI 覆盖的空带 = 局部黑边），改为贴物理全屏——scale 按物理宽/高算，
  刘海带被画布覆盖、背景满铺贴边。避让交给 CSS：新增
  `env(safe-area-inset-top/right/bottom/left)` 作为长度值来源（任意长度属性
  可用，按 design px 换算），元素用它写 padding/inset 自行避开 notch /
  home indicator（web `viewport-fit=cover` 语义）。letterbox 不变（black
  bars 已让位，env() 恒 0，不重复避让）。**Breaking**：Driver 的
  `_safeArea` Inspector 开关移除（语义统一后它是第二真理源）；fit 模式下
  vw/vh 分母变为含 unsafe 带的完整画布（100vh = 物理全高）。
- **视口单位放开到全长度属性（#110）**：`vw` / `vh` / `vmin` / `vmax` 此前
  只收尺寸族/inset/margin，现放开到 `font-size` / `padding` 族 / `gap` 族 /
  `letter-spacing` / `border-radius`（响应式字号 `font-size: 2vmin` 由此
  而来；`%` for font-size 仍拒）。字号/字距的视口值沿继承链传播 resolved
  px。`inset` 四边简写进围栏。pkg 格式 v51（旧 pkg.bin 拒载，须重打）。


### Fixed
- **HTML `draggable="true"` 打包即丢——声明式拖拽使能无效（#75 验收批）**：组件
  展开路径（真实打包编排 `bridge_with_components`）的元素构造把 `draggable`
  硬编码 false——旧 `bridge()` 只有单测在用，测试绿但所有页面节点打进 pkg
  都是 false，运行时永远拖不动（声明式拖块纹丝不动；运行时 `Node.Draggable`
  开关走 FFI 直写不受影响）。现页面元素与组件 host 两个构造点都按 bridge
  主路径同款口径提取（fence 值域 true|false，非 "true" 按 false 兜底）。
- **文本框删空后屏幕残留最后一个字符（#76 验收批）**：layout 期的控件文本
  measure 在显示串变空时跳过缓存写入但不清旧缓存——渲染层的 lazy fallback
  只在缓存为空时触发，读到上帧 value 的残留 TextLayout，已删除的旧文本继续
  上屏（长按 Backspace 删空后 'a' 留在框里）。现空显示串回落 placeholder
  同源缓存（无 placeholder 才清缓存）：删空即画 placeholder（此前删空后
  placeholder 也不显示，同一根因），布局测高继续读缓存算行高（不变）。
- **进播放时的 URP「camera overlay no longer exists」警告消除（#109 验收批）**：
  编辑态（`[ExecuteAlways]`）Driver 把 DontSave 相机挂进宿主 Base 相机的
  cameraStack——该 stack 随场景序列化，而 DontSave 相机不随场景进播放副本，
  引用变悬挂；URP 首帧渲染 Base 相机时报一次警告并自清。挂载前先清 stack
  里的死引用（场景加载 Awake 期执行，先于首帧渲染），警告不再外溢。
- **关闭双 Stage 小窗不再清空整个屏幕 UI（#109 验收批）**：两个 Driver 共享
  同一台 hub `YioUICamera`，但小窗 Driver 销毁时无条件把相机从宿主 Base 的
  URP cameraStack 摘除——Overlay 相机不在任何 stack = 整机不渲染，主 Stage
  的面板/血条/全部屏幕 UI 消失（3D 挂载面板走场景相机幸存），直到 resize
  触发重配才恢复。摘除现在带引用计数判据：相机仍被其它 Driver 持有时只减
  引用不摘 stack（`YioStageHub.CameraHeldByOthers`），最后持有者随相机销毁
  摘除。
- **孤儿共享相机清扫——「看不到 3D 场景 / 相机一片黄」的真根因（#109 验收批）**：
  Driver 是 `[ExecuteAlways]`，编辑态 Awake 也会建 `YioUICamera`
  （DontSaveInEditor）；domain reload 不跑 OnDestroy，幸存相机被 Unity 挪进无
  效场景，hub 的按名认领（扫 caller 场景根）永远看不到它。孤儿以 Base 型 +
  depth 0 压在宿主 3D 相机之上每帧重渲染：clear=Depth 时把宿主 3D 输出整帧抹
  成自己的底色（世界锚点/压测页看不到 3D 场景），渲染目标未初始化时整屏垃圾
  色（相机一片黄）。`AcquireCamera` 现在认领/新建后清扫：销毁无效场景里的
  DontSave 同名相机（其它已加载场景的合法共享相机与用户手建的尊重不动）。
- **UI 相机叠加改走 URP cameraStack（#109 验收批）**：URP 的 Base 相机没有
  「保色叠加」语义——Depth 把颜色也清掉，Nothing 读到未初始化缓冲。有宿主打
  底相机时 UI 相机配成 Overlay 挂进宿主 Base 相机的 cameraStack（Base 先画
  天幕/3D，Overlay 后画不碰颜色/深度初值），反射操作不硬引用管线程序集；
  Built-in 回退经典 Depth；无打底相机（纯 UI）用 SolidColor。OnDestroy 从
  stack 摘除，避免悬挂条目。
- **滚动/世界锚点跟随的渲染增量冻结（#109 验收批）**：mesh 合并批此前矩阵恒
  IDENTITY + 成员 payload hash 为局部系位移不变量——纯滚动 / Transform
  平移变更下批的双轴 hash 全不变，整批被判 Skip，后端镜像冻结在旧位置（拖动
  滚动容器时布局错乱、世界锚点跟随一卡一卡）。合并批改为持 anchor 平移矩阵
  （同质批平移 → Header 级只挪 GO）+ 整批 payload hash 定级（混合批净位移 →
  Full 重传），滚动中所有合并批严格跟手；稳态帧仍全 Skip（增量效率不变）。
- **`SetNodeRenderVisible` 同值重复写不再触发挥重建（#109 验收批）**：世界锚点
  路径每帧对屏内节点调 `visible=true`——旧实现每帧 bump
  `render_input_version`，增量渲染指纹全部 miss（缓存失效 churn）。同值写入
  幂等短路。
- **运行时第二 Driver（双 Stage）此前收不到任何输入（#109 验收批）**：运行时
  `AddComponent<YioStageDriver>` 拉起的小窗 GO 上没挂 `YioInputCollector`——
  hub 路由探测（`PointerHitProbe`）与输入采集都吃它，缺 collector 时小窗永远
  轮不到输入所有权，点击全部穿透到底层 Stage 的按钮。小窗 GO 现在配好自己的
  collector。配套：**stage 文档根不可命中**（`create_root` 建的宿主容器
  `touchable=false`）——根铺满画布且可命中时，「点到空白处」会命中根，多 Stage
  输入路由据此把指针下所有底层 Stage 饿死（小窗外的按钮也点不动）；overlay 类
  Stage 的页面根另应声明 `pointer-events:none`（交互面板再 `auto`），把命中面
  收窄到真正的交互内容。
- **showcase：压测页不再遮挡场景与按钮（#109 验收批）**：页根背景改透明（3D
  场景从 UI 底下透出，与世界锚点页同款）；500 血条网格让开左侧面板区且模板根
  `pointer-events:none`（纯展示条不抢命中——此前整片网格盖住面板，隐藏/跟随
  按钮点不到）；生成血条同步生成 500 个 3D 方块（血条的 3D 对应物，投影跟随
  是否正确肉眼可辨）。
- **showcase：适配页补返回栏、首页全屏覆盖适配（#110 验收批）**：adapt 页补
  `<page-top>` 返回首页（其余页同款）；首页 nav-grid 吸收画布富余高度（footer
  落底，拖高窗口不再留死区）+ 页根允许滚动（窗口拖矮时底部列表被屏幕裁掉 →
  改为可滚动）。
- **YioUICamera 裁剪窗扩为 UI 平面中心的前后对称大窗（#102）**：此前
  `near=0.1 / far=100`——NativeHost 3D 内容按 design px 归一化（数百 px
  高 × root scale 即数千世界单位深），居中摆位时深度越过 UI 平面向后延
  伸到相机（z=-10）之后，旧窗口把模型后段/远端整片裁掉（视觉上只剩贴
  着 UI 平面的五官或整段消失）。改为 `near=-9990 / far=10000`（正交相
  机支持负 near），前后各留万级深度余量。UI mesh 全在 z≈0 平面、排序走
  sortingOrder，均不受此窗口影响。
- **UI layer 改用内置 "UI"(5)，杜绝宿主 layer 冲突（#105）**：此前 Driver
  硬编码 layer 6——6–31 是用户可命名层，宿主工程把它命名成自己的用途
  （如 FloatingText）完全合法，后果是语义混用 + Everything 掩码主相机把
  UI 再画一遍（双影）。改用 Unity 内置锁定层 "UI"(5)（0–5 不可改名，
  冲突类结构性消灭；与 FairyGUI StageCamera 同选）。宿主侧只需标准 Unity
  惯例：3D 相机 cullingMask 排除 UI 层。曾手工排除 layer 6 的工程改为
  排除 layer 5 即可。
- **编辑态自建 UI 相机不再序列化进场景（#108）**：`[ExecuteAlways]` 下
  Awake 自建的 YioUICamera 无 DontSave 标记、且把引用写进序列化字段
  `_uiCamera`——编辑态保存场景把相机 GO 烤进场景文件，换设计分辨率后
  场景里残留旧适配缩放 + 跨场景悬空引用。自建相机改走 `[NonSerialized]`
  独立字段（序列化字段只承载用户指派意图）+ `HideFlags.DontSaveInEditor`
  （对齐 NativeHostManager 先例），`OnDestroy` 主动销毁（DontSave 物
  Unity 不接管回收）。存量场景里已烤入的 YioUICamera GO 删一次即净。
- **`yio build` 省键化可选 manifest 字段（#103）**：`design` / `match_mode`
  未设置时 `yio.runtime.json` 此前写出字面 `null`，Unity 侧手写 reader
  （`YioManifests.cs`）对 null 直接抛解析错——manifest 整体作废，包列表随之
  丢失，下游报出离根因三层的笼统 `instantiate failed`。现在 None 一律省键
  （字节级测试锁定），兑现「缺项 = 引擎侧兜底」的既有契约。同时 manifest
  解析失败升级为阻断错误：`LogError` + 中止装 UI（此前 warning + 继续，
  包加载不到还往下跑只会产出更糊涂的下游报错）。
- **preview 文档根满高（#107）**：preview 注入的 `base.css` 补
  `html, body { height: 100% }`。浏览器文档的 html/body 默认高度 auto，页面
  根容器写 `height:100%`（浏览器习惯写法）会塌成内容高——预览纵向溢出、
  底部错位，check 零警告，预览与运行时行为分叉。补满高后预览的
  containing block 语义与运行时 stage 对齐（与外壳 shell 自身写法一致）。

### Added
- **preview 按工作区字体注册自动注入（#104）**：server 给每个服务页在
  `<head>` 开标签后注入 `<style id="yio-preview-fonts">`——fonts 段每字体
  一条 `@font-face`（`src: url(/ws/<file>)` 绝对路径，任意页面目录深度可解
  析）+ `default` family 一条 `body` 规则（镜像 core「未声明 font-family →
  默认字体兜底」语义——此前浏览器 UA 默认 serif，无 font-family 文本在预览
  与运行时分叉）。注入先于页面自身样式，工作区 CSS 恒可覆盖默认规则。
  `.ttc`（浏览器不支持 TrueType Collection）与磁盘缺失的注册字体跳过 +
  stderr 告警——排版失真不再无提示。recipes.md 的手写 `@font-face` 教程
  降级为「仅覆盖时需要」（不同源文件 / 超大字体换 `font-display: swap`）。
- **字体管理命令闭环 + 单一默认契约（#106）**：`yio font remove <family>`
  （摘注册；`fonts/` 下源文件保留——文件可能由人手管理，摘除后成孤儿由人清）
  与 `yio font default <family>`（设为唯一默认）。`--default` 语义改为互斥
  转移：新设默认自动摘除旧默认，不再产生双默认 workspace（运行时取哪个未
  定义）。未注册 family 报数据错（exit 1）。
- **`yio verify`：Unity batchmode 导入冒烟（#99）**：build 重打 → 拉起工程绑定的
  Unity 编辑器（batchmode）→ 包内 `Yio.Editor.YioVerifySmoke.Run` 做 Refresh +
  逐文件正向加载（png → Texture2D、其余 → 非空 Object）→ 解析报告。把「产物可
  导入」从人肉开 Unity 变成一条命令（发版本地验收面）。编辑器三层查找：
  `--unity-editor` 参数 → `ProjectVersion.txt` 匹配 Hub 标准目录 → exit 2 教
  用法；编辑器路径不进入库共享的 config。退出码：0 全过 / 1 导入失败（逐资产
  诊断）/ 2 工具性失败（工程被编辑器占用、超时 15min、executeMethod 未运行）。
  分层：本地导入冒烟归 verify，CI 的 EditMode/PlayMode 测试归 unity-smoke
  workflow（等 license 启用），两层不重复。
- **画序声明完整性双门（#101）**：E1（error）= z-index 声明在非定位、非
  flex item 元素上直接拒绝——浏览器对该声明视而不见而运行时恒生效，预览
  会说谎；围栏里写不出来的分歧够不着，运行时 API 直改 z 不受影响。
  W1（warning）= 同父兄弟 static 与 positioned（或声明 z）混排、static 侧
  无显式 z——positioned 元素恒画在 static 之上（与树序无关），漏声明靠
  「碰巧画对」正是 #96/#100 的成因；补 `position:relative; z-index:0`
  （同视觉、显式意图）消警。两门同看 inline + 结构匹配 class 规则
  （`.overlay{position:absolute}` 类写法计入，运行时可变属性选择器规则
  跳过防误报）。存量内容影响：真实工作区 0 error（class 规则视野消除了
  最常见误报形态），showcase 存量 28 处 W1 warning（不拦构建，债可见化）。

### Added
- **CLI ↔ Unity 包版本漂移自动检测（#80）**：`yio check` 顺 config 的
  `unity_root` 读 `Packages/packages-lock.json`，`com.yio.unity` 版本与
  CLI 版本不一致时告 `YioVersionDrift` warning（双向：CLI 落后指向刷新
  `.yio/` 的 exe，CLI 超前指向更新 Unity 包——`format_version` 只增）。
  支持三种 lock 形态：裸 semver / `file:` 本地引用（顺路径读目标
  package.json）/ git URL（取 `#v` 片段，裸 commit hash 无法离线定版则
  跳过）。无 `unity_root`（本地模式）或 lock 无条目（未装包）不检。

### Fixed
- **progressbar 填充比例 ARIA 语义化（#97）**：`aria-valuemin` 此前不参与填充
  数学（core 按 `value/max`，ARIA 标准是 `(value-min)/(max-min)`，min≠0 时
  填充偏少）。`ControlInit/ControlState::Progress` 补 min 字段（pkg 格式
  v49，旧包拒绝——Unity 包与 yio.exe 同版本重打），打包期解析
  `aria-valuemin`（缺省 0，min=0 行为不变、零迁移）；FFI `set/get_control_min`
  对 ProgressBar 开放（此前返回 -1 的「无 min 语义」禁区废除），C# `ProgressBar.Min`
  可读写；运行时合成 `aria-valuemin` 属性镜像（改 min 后 CSS 属性选择器同拍
  生效）；preview `controls.js` 同口径对齐。

## [0.0.17] - 2026-08-28

### Fixed
- **pkg 版本门修复：MIN_VERSION 漏拍 v48（0.0.16 起 CI 红的真因之一）**：v48
  bump 只抬了 MAX、MIN 停在 47——v47 旧包漏过版本门，以 Bincode 结构错配炸成
  无指引的「malformed pkg.bin」（rc -1），而非 rc 1 的「Unity 包与 yio.exe
  同版本重打」专属文案。MIN 已钉回 48 并加护栏测试（`min_version_tracks_current`
  ：MIN/MAX 必须恒等于当前版本——历史不变量：每版 bump 都改 bincode 布局）。
  同批把 HeadlessTests 的 13 个 fixture 包重打到 v48（此前是 v47 陈货，
  `dotnet headless` CI 门自 0.0.16 起全挂）。流程堵漏：`xtask reout` 新增
  fixture 重打步骤（`<name>.workspace` → `yio build` → 拷回 `.pkg.bin` →
  清构建现场），与 showcase bundle 同一「无条件重打 + 字节对比幂等」纪律。
- **core 画序升级为 stacking context 全局分层（#100 分歧粒度修复）**：0.0.16 的
  画序分层是**逐父兄弟排序**——嵌套在 static 子树里的 opacity<1 / transform /
  filter / 定位+声明 z 后代不会上提，整棵 static 子树（含半透明图标）沉在
  absolute z0 底图之下；浏览器按 CSS Color 规范把 opacity<1 的 static 元素
  「当作 z-index:0 的 positioned 元素」绘制（Chrome 像素取证：static 顶栏中
  opacity .65 图标浮在底图上、同位置的文本 span 被盖——Tripawd 实测的混合
  结果）。core 现按 CSS Appendix E 语义做 **SC 全局分层遍历**（新
  `scene::stacking::paint_order`）：每个 stacking context 内「负 z SC → static
  树序 → z0 层（positioned z:auto + opacity/transform/filter SC）→ 正 z」，
  SC 后代从任意深度上提、SC 内部递归分层（子树整体移动不变量保持）；render
  主 DFS（`assign_sort_keys` 拆结构/画序双 pass）、open popup 追加、hit 逆序
  遍历三消费点共用同一份序。同批语义对齐：flex `order` 现在也改**画序**
  （order-modified tree order，浏览器同序；此前 render 侧不排 order）；多 root
  命中序改为后 root 优先（与渲染序「后画在上」同向，修掉 hit 与自己注释矛盾
  的旧顺序）。已知口径分歧（一处，fence.md 记档）：非定位、非 flex item 元素
  上的声明 z-index，浏览器忽略、core 恒生效（运行时直改 z 的 fgui 血统语义）。

## [0.0.16] - 2026-08-28

### Fixed
- **preview 静态资产 revalidate 化**：`/ws/` 静态资产（字体/图/
  CSS/JS）从无差别 `no-store` 改为 `no-cache` + `Last-Modified` + 304 再验证——
  活文件语义不变（每次导航校验 mtime、改动即刻 200 新字节），但不再重传。
  背景：工作区 25MB 级字体在 no-store 下每次导航全量重传，把 @font-face
  `font-display: block` 的隐形文字窗（布局占位在、字形不画、`!important` 无效
  ——非级联问题）拉成每次刷新必现；HTML 恒 no-store（注入产物依赖外部脚本
  存在性）。recipes.md 补消费侧指引（超大字体首载窗建议 `swap`）。
  （#96 排查途中修的独立缺陷——非 #96 根因，见下条。）
- **core 画序改采 CSS painting order 分层语义（#96 终局根因修复）**：Tripawd
  battle 页顶栏文字与圆钮底「预览不画、运行时画」的真因——`.paper-bg`
  （`position:absolute; z-index:0`）+ static 的 `.topbar`：旧 core 画序只按 z
  数值稳定排序（全员缺省 0 → 纯 DOM 序，内容在上）；浏览器把 positioned 元素
  与声明了 z-index 的 flex item 整体画在 static 内容之上（底图盖内容）。core
  现按分层键 `(tier, z)` 绘制/命中（`ResolvedStyle::paint_key`）：负 z 定位
  沉底 → static 内容 → z=0 定位/声明 flex item → 正 z；render
  `paint_order_children` 与 hit `effective_draw_order` 同键镜像。**迁移注意**：
  依赖「DOM 序隐式压住 absolute 底图」的页面运行时观感会翻转为浏览器语义
  （本来在浏览器/预览里就是被盖的）——要内容压底图给内容 `position:relative`
  + `z-index`（Tripawd `.topbar` 已按此修）。pkg 格式 v48（ResolvedStyle 加
  `z_declared` 位，旧包拒绝）；`z_declared` 与 z_index 同进同出（mapping 置位、
  dynamic 双字段覆写），非 0 的 z 本身即视为声明（运行时直改路径兼容）。

## [0.0.15] - 2026-08-28

### Changed
- **光标皮肤去内置，改消费侧注册（#93 followup）**：`YioStageDriver` 删除内置手型
  像素画——intent 0/1（箭头/手型）缺省均为系统光标；新增
  `SetCursorTexture(uint intent, Texture2D texture, Vector2 hotspot)` 按意图注册
  消费侧贴图（null/已销毁 = 清除；作用于当前激活意图时立即重放；贴图所有权归
  消费者，driver 只销毁自建载体）。intent 2（cursor:none）保留内置全透明载体。
  hotspot 从纹理左上角量（Unity docs 约定）。showcase 手型像素画搬入
  `ShowcaseRunner`（unity/showcase-unity 与 -2021）作注册示例。

### Fixed
- **手型光标纹理上下颠倒 + 热点错位（#93 followup）**：`SetPixels32` 行序
  bottom-up（下标 0 = 左下角像素）而几何按 y=0=顶 生成——写入按 (S-1-y) 翻行；
  `Cursor.SetCursor` 热点从纹理左上角量（此前误按 bottom-up 换算），修正为食尖
  (12,1)。两个坐标系约定独立，勿混用。
- **preview 组件样式作用域根类规则失配（#95）**：组件 `<style>` 的浏览器作用域
  改写收编进 Rust 单实现（server 新路由 `/yio-preview/comp-style/<name>.css`，
  fence 同一入口抽样式），每条规则输出「后代 + 根匹配」双分支——core 语义
  「作用域 = 子树含模板根、选择器原样」在浏览器侧全等兑现，模板根自身类的规则
  （`.tip { … }`）不再整条静默失效。同批口径对齐：@keyframes 同名碰撞宿主优先
  （注入次序 base < 组件 < 页面）；非 @keyframes at-rule（@media 等）与围栏外
  选择器在 preview 同步丢弃 + 注释（fail-closed，不产「预览能看、构建报错」的
  假象）；组件 `<link rel=stylesheet>` 与声明块 `url()` 相对路径按组件文件位置
  归一；spinbutton 缺省 aria-valuemax/min 无界（旧版缺 max 时钳死到 0）；
  progressbar 对齐 core 的 (value/max) 口径。expand.js 的正则 CSS 前缀器删除
  （#94 前半步：CSS 语义单真相在 Rust，客户端 A 层收敛为纯 DOM 机械层）。
  #96 同批归因为 #95 家族（组件模板根类规则失配）并顺带补上宿主**静态** class
  的初始镜像（原 MutationObserver 只看后续变更，HTML 写死的 `class="sub"` 类
  宿主链静态命中语义在预览里缺失）。
- **check 长度形态门（#95 评论区请求）**：长度族声明（`Length`/
  `LengthPercent`/`LengthPercentAuto`/`BorderRadius` 属性 + `padding`/`margin`
  简写）打包期逐 token 校验单位——非零长度必须带单位，仅 `0` 可裸写。此前
  `padding: 14px 6 16px 6` 一类声明零警告过检：core 对裸数字当 px（运行时
  生效）、浏览器整条丢弃，预览与运行时静默分叉。单位集按域开放（px-only
  通道收 px；尺寸族/inset/margin 收 px/%/视口单位/auto；border-radius 收
  px/%）；longhand 只收单值（多 token 浏览器无效、core 只取首值）。`em` 全域
  不在围栏（core `parse_px` 不收、浏览器收的反向分歧同样拦）——showcase
  home 页 4 处 `letter-spacing` em 已按 font-size 换算 px（此前预览有字距、
  运行时无的活分歧）。

- **消费侧文档漂移门 + 补漏**：新增 `consumer_doc_sync` 测试把 fence 的
  doc↔schema 交叉校验延伸到随 scaffold 分发的模板文档（fence-schema.md 标签表
  双向精确集 + css-reference.md 属性全量覆盖）。首跑抓到两处存量漂移并已修：
  fence-schema.md 标签表缺 `<a>` 行（#74 加链接时只更新了导语）；壳标签
  （html/head/body/title/meta/style/link/script）整组未进消费侧文档——已补
  完整表格。措辞级漂移由 AGENTS 新增的「文档涟漪表」人工兜底。

## [0.0.14] - 2026-08-28

### Added
- **桌面指针 affordance（#93）**：悬停 pressable 控件（button/tab/toggle/radio/
  slider/dropdown/option 与 `<a>` 链接）软件指针自动变手型——UA 默认行为，作者
  零声明；命中细化到控件内文字/内联子后沿祖先链上溯宿主控件判定（悬停按钮
  文字同样手型），disabled/不可命中控件不给手型并截断。新围栏属性 `cursor`
  （`auto` 缺省 = UA 默认 / `pointer` 手型，标非控件可点区 / `default` 箭头 /
  `none` 元素级隐藏——作者显式声明恒压 UA 行为，浏览器一致）。pkg 格式 v47
  （旧包拒绝加载；同批 flags 字节新增 disabled 位，布局不变）。HTML 布尔属性
  `disabled`（button）接通运行时 disabled 态——此前过围栏但运行时无人消费
  （禁用按钮悬停仍手型、点击不被抑制），现映射既有 disabled 语义（点击抑制 /
  `:disabled` 伪类 / 光标 affordance）。Unity 侧 Driver 订阅
  `YioHost.CursorIntentChanged`（0 箭头/1 手型/2 隐藏），手型纹理程序化生成（按
  Unity 光标纹理要求构建：RGBA32 / 保持可读 / 无 mip 链 / 标准 32×32 尺寸——
  隐藏载体初版 4×4 被 Windows 硬件光标拒收）、Destroy 还原系统光标；自定义软件光标纹理帧业务仍走
  `Cursor.visible=false + 自绘 sprite` 的既有方案。
- **preview 行为层分层（#92）**：组件展开/控件语义/结构性 polyfill 收编为框架
  真相副本（嵌在 yio 二进制），preview server 对每个 HTML 页**恒注入** A 层 boot；
  消费侧脚本（B 层：演示数据/导航/页面交互）经 `/yio-preview/lib/*` 绝对 URL
  import，不再拷贝 showcase 参考实现——第二真相源腐烂路径（宿主态选择器失配/
  keyframes 帧选择器误杀/#90 式契约漂移不同步）结构性消除。Tripawd 三处狗粮
  补丁已吸收进真相副本（宿主态镜像到模板根 + `@keyframes` 内部原样放行）。
  静态文本响应补齐 `charset=utf-8`（无 charset 时中文按本地编码解码乱码）。
  展开层完全收编进 Rust 单实现开 deferred 票 #94。

### Changed
- **品牌更名 LoomGUI → Yio（#91）**：crate 前缀 `loomgui_*`→`yio_*`、CLI `loom`→`yio`、
  C# namespace/类型 `LoomGUI.*`/`Loom*`→`Yio`（一步到位不留过渡 alias）、
  dll `loomgui_ffi_c.dll`→`yio_ffi_c.dll`、UPM 包名 `com.yio.unity`、
  工作区协议 `.loom/`→`.yio/`（`yio.workspace.json` / `yio.runtime.json`）、
  CSS 注释锦点 `@loom-hook`→`@yio-hook`、preview 关停协议 `X-Yio-Token` / `/_yio/shutdown`。
  **pkg/frame 魔数字节不变**（既有 `.pkg.bin` 兼容）；FFI 导出符号改名并与 C# 绑定同批再生，两端一致。

### Added
- **CSS IFC 换行控制全集（#73）**：
  - `white-space` 扩到五值：`normal` / `nowrap` / `pre` / `pre-wrap` /
    `pre-line`（空白折叠 × 自动换行 × 源换行保留 三轴组合，此前仅
    normal/nowrap）。pkg 格式 v45（旧包拒绝加载）。
  - 新属性 `overflow-wrap`（`normal` 超长词溢出不拆 / `break-word` 词独行
    仍超行宽才逐字拆）、`word-break`（`break-all` 任意字符可断 /
    `keep-all` CJK 词内不断）、`text-wrap`（只收 `nowrap`；CSS Text 4 的
    `balance`/`stable`/`pretty` 围栏拒绝——标题居中用 `text-align`）。
  - **CJK 禁则（kinsoku，原 #51 并入）**：断行自动避头尾——行首不出句读/
    闭括号、行尾不出开括号，违规断点自动左/右移（悬挂标点不做）。
  - 语义对齐浏览器的连带修正：静态文本空白串折叠为单空格、源换行在
    normal/nowrap 下折为空格（此前 plain 路径直接断行）、超长词默认不再
    逐字拆（需显式 `overflow-wrap:break-word`）、rich-text-block 的
    white-space 声明真正生效（此前被忽略）、换行后行首悬挂空格移除。
  - 文本控件（TextField/TextArea/NumberField）空白语义冻结 pre 系
    （空格/换行原样保留，光标字节映射不受折叠影响；CSS UA 同为 pre 系），
    换行开关仍尊重声明。
  - showcase lab §18 摆台（11 盒 × 各值 × 禁则段落，判据见页内）。

## [0.0.13] - 2026-08-27

v0.0.12 后一批：动画引擎终态基建（#9/#10，core 动画通道 + C# TweenBuilder
全接线）+ `yio preview` 本地预览工作台 + 稳态帧文本换行回归修复 +
AI 安装手册重写；发版轮收尾批（transition 首帧修复 / 预览体验三连修 /
agent 文档补齐）；狗粮反馈批（#85-#88：MeasureText 文本测量 / F8 dump
归因增强 / 运行时告警面 / 动态内容范式文档 + lab §17 用例）；验收后
文档漂移批（#90：分发文档与 check 对齐 + 双向对账测试防再犯）。

### Fixed
- **工作区分发文档与 check 漂移批（#90，Tripawd 0.0.13 验收实证）**：
  - css-reference 的 transition 支持集从过时的 4 通道修为实际 8 项
    （width/height/flex-grow/box-shadow 自 #10 起可过渡），并纳入
    `fence-sync` 锚点块与 `TRANSITION_PROPS` **双向对账测试**——少写/多写
    通道即 CI 红；顺带修同段「transform-origin 不存在」的陈旧声明
    （#9 已交付）。
  - fence-schema 的 combobox 契约行补 `data-slot=value` 必需子（照抄旧
    文档直接 build 失败）；patterns.md canonical CSS 同步补 value 锚样式
    行与措辞。新增「`REQUIRED_CHILDREN` ↔ role registry 行」对账测试。
  - fence 错误信息 6 处 `docs/design/fence.md §x` 死链（工作区拿不到的
    仓库文档）改指 scaffold 分发的技能文件（fence-schema.md /
    patterns.md / css-reference.md）。
  - api-reference 补「Runtime diagnostics」段：`DumpSceneTree(filter)`
    签名与读法（`lh=NN.00x` 等直读信号）、F8 `[Scene tree]` 段、
    overlap=0 滚轮告警语义；ProgressBar.AnimateValue 注记不再声称
    「CSS transition 做不了 width」；Value domains 补 `line-height`
    单位域（无单位=倍数的陷阱）；CustomElement「打包期类型、C# 侧
    `Instantiate("stem")` 拿普通 Container」等式补全。
  - 仓库内 fence.md 两处内部矛盾顺手修（诊断表「四通道」→ 全集表述、
    Dropdown value 锚「可选」→ 必需）。
  - 随 yio.exe 分发；消费侧 `yio scaffold` 刷新技能即可拿到新文档。

### Added
- **文本测量公共 API（#86）**：`UIContext.MeasureText(text, family, sizePx,
  maxWidth)` → `TextMetrics{W, H, LineCount}`——布局前纯文本预估（tips 预分行 /
  飘字宽估 / 按钮自适应宽），与 solve 内文本测量同一条断行代码，预估即所见；
  `maxWidth > 0` 按宽断行，缺省单行。family 未注册抛 `UIContractException`
  （不静默 fallback 到默认字体——拿错字体估宽没有意义）。消灭业务侧手数字数
  （Tripawd tips「17 字/行」类魔法数字）。
- **F8 dump 布局归因增强（#85）**：scene JSON 文本节点附 resolved 块
  （font-size / 行高乘数——`line-height:26` 被当 26 倍乘数类问题从反推变直读 /
  行数 / 每行宽），滚动容器附几何块（viewport/content/overlap/pos/物理）；
  新增可读树视图 `YioHost.DumpSceneTree(filter)`（每节点一行
  `tag#id.class rect` + 文本/滚动关键值，ASCII 树缩进；filter = id/class
  子串只出命中子树），F8 诊断输出接入 `[Scene tree]` 段。
- **运行时告警面（#87）**：滚轮打进「声明 overflow:auto/scroll 但内容未溢出
  （overlap=0）」的容器 → warn-once 一条 Console 告警（#64 类「滚动容器无效」
  从半天排查变一行日志）；诊断订阅（缺字报告 / 运行时告警）改
  Editor/Development build 门控——发布 build 零日志成本。核心告警通道
  （`Scene.warnings` → `take_warnings` FFI → `RuntimeWarning` 事件）既有，
  本批补检测规则与门控；transition 通道白名单打包期 warning 既有（fence
  `TRANSITION_PROPS`），过期注释一并修正。
- **动态内容范式文档 + lab §17 用例（#88）**：runtime skill 新增「Dynamic
  content paradigm」章节——何时模板实例化、何时 `Create<T>`、坐标数据与结构
  样式的分工；动态类（`dyn-*`）的声明位定为独立 css 经 `<link rel="stylesheet">`
  引入（围栏可校验 / 随 pkg 打包 / 预览可见，零新机制）；伪类（:hover /
  :nth-child）对实例化节点照常生效的适用面声明。showcase lab §17 摆台验证：
  `GetTemplate` 实例化 ×3 + Query 注入 + `dyn-selected` 类切换（读 computed
  背景色进读数）+ hover/斑马纹判据；`lab.dynamic.css` 同批入库。
- **动画引擎终态基建（#9）**：ease 全集、统一 `TweenValue`、tween 池化、
  percent keyframes（#77）、transform-origin；C# 侧 `TweenBuilder`
  fluent wrapper + `TweenComplete` 标签路由 + lab §16 运行时用例
  （pkg v43）。
- **layout & box-shadow 动画通道（#10）**：同域端点插值，box-shadow
  渐变动画；C# `TweenBuilder` layout/box-shadow 接线 + showcase
  layout-anim 页（pkg v44）。
- **`yio preview` 预览工作台**：CLI 新子命令——起本地 server 供人工
  浏览器预览设计工作区；showcase 预览栈迁移其上（ESM 入口改写）。
- **AI 安装手册**：`docs/ai-setup.md` 重写安装链路（两问流程——输出目录
  也询问）；README install 章节同步改版。

### Fixed
- **稳态帧文本换行回归**：稳定帧必须携带 text_layouts 下发渲染——修
  高帧率稳态下长文本换行丢失的回归。
- showcase 不定态进度条滑动 keyframes 从 percent 改 px（percent
  translate 被静默跳过不动画）。
- **transition 首帧闪现终点值**：transition drain 提交 tween 时未写
  `scene.anim`，提交帧 solve 读到的是级联终点——展开面板先满高一帧再
  塌回起点起播（反向则先消失一帧）。现提交即以 n=0 预写起始值（与
  animation player 的 backwards 首帧立即写同纪律）；delay 期间持有
  起始值亦合 CSS 语义。回归测试
  `transition_first_frame_holds_start_value_not_endpoint`。
- **preview shell 适应窗口**：设备框整体等比缩进视口（观察级
  transform，不触发 iframe reflow，页内仍按设计分辨率渲染——保真语义
  不变），默认开启、上限 100% 不放大；顶栏「适应窗口」可切回 1:1 像素
  检查（此时恢复滚动）。修「窗口小于 1920×1080 就必须滚动」的预览体验
  问题。随 yio.exe 发布。
- **preview：去掉 preview-base.css 的 body 卡片包装**（旧 file:// 直开
  时代的 `padding:24px`+居中美化）：iframe 视口即设计视口，包装会把
  1080 高页面顶成 1128 出 iframe 内滚动条，且 .root@(24,24) 与运行时
  (0,0) 不符。rect-diff 的 reset.css 本就按无包装对齐，此改动使人类
  预览与对齐门同语义。
- **showcase layout-anim 验收工效**：#1 折叠面板的 320px 参照宽条移到
  面板正上方、200px 参照高条贴面板右侧（#6 的 220px 终点参照同理）；
  #1 内层容器改纯 div（去掉借 `.stage` 类残留的 `flex-wrap:wrap`，
  规避 core 列方向 wrap 容器动画误换列问题 #82——根因已定位，见 issue）。
  另补 `preview/pages/layout-anim.js`：预览里四个验收按钮可驱动。
- **agent 文档补齐（#84）**：runtime api-reference 的动画章节不再声称
  「没有命令式补间」，补 `TweenBuilder`/`TweenChannel`/`EaseKind`/
  `TweenShadow` 完整签名与语义（随 skill 模板经 yio.exe 分发）。

## [0.0.12] - 2026-08-25

v0.0.11 后两波：狗粮残留批（#47/#49/#50，公共 API 投影缺口补齐）+ M3 P0 开工批
（#40/#26/#29/#58，跨引擎预备的还债与性能地基）+ 发版前 review 修复批（2026-08-25
代码审查产出：#66 修复的幂等性雷、文本导航字节映射方向、拖选门控、tabpanel
打包期门、投影层三小修；MirrorPool EditMode 测试同步升 v14 blob）。

### Added
- **不定态进度条（#47）**：`aria-indeterminate` 合成属性 + fill 宽度让位——控件
  状态驱动，零打包配置。
- **TextField 键盘编辑（#49）**：词级导航/删除（Ctrl+方向/Backspace/Delete）、
  TextArea 行导航（Home/End/上下 + sticky x）、鼠标拖选路由仲裁。
- **投影层缺口批（#50，7 项）**：MaxLength 属性接线、`OptionItem.Index`、
  `Node.Computed`（NodeComputedStyle 只读视图）、`LongPressEvent` 类型化
  （core 产 EventType 9 但 demux 此前跳过）、`Node.SetPointerCapture`（DOM
  setPointerCapture 对等，Up 自动释放）、`StopImmediatePropagation` 复活
  （EventBus 重写时丢）、`Node.CancelClick`（配 LongPress 的长按取消）。
- **border/背景共存打包 warning（#58）**：彩色边框与 background-image/gradient
  共存时互斥不画（render 层既有限制）——`yio check`/build 现在当场点破
  （`BorderBgExclusive`），不再让作者猜。
- **tabpanel 打包期门（review 批）**：`role="tabpanel"` 手写内联 `display:none` →
  `FenceTabpanelHiddenByAuthor` error。显隐所有权归 TabList 运行时（激活面板靠 unset
  inline display 回落作者样式），作者内联 none 烙进 base_style 后 unset 清不掉——激活
  面板永久隐身的静默坏，存量写法打包期点破。另 fence.md 补「运行时合成属性不参与
  打包期 CSS 命中」：只写 `[aria-indeterminate="true"] [data-slot=fill]` 一类态规则
  会吃 `FenceControlChildWithoutCss` 假错误，子部件须另有命中打包期 HTML 的基础规则。
- **solve 基准**：首个 criterion bench（`cargo bench -p yio_core`，
  api-infra 形状 ~2400 节点三组对拍）。

### Changed
- **BREAKING：NodeId ABI u32 → u64（#26）**：位型 = index:32 + generation:24 +
  tag 字节:8（tag 字节 = 渲染合成 id 命名空间：shadow 层/文本跨页子页/TF 合成层/
  scrollbar thumb 各占区段）。frame blob VERSION 13→14（node_id/parent_id 列
  4B→8B）；C# 绑定与 Runtime 全量重生成。包内 .dll/Bindings/Runtime 同 commit
  配套升级；绕过包直接 P/Invoke 的原生宿主须同步。合成 id 的 4096 节点硬上限与
  generation 12-bit 回卷上限（4096 代 → 1670 万代/槽）一并消灭。
- **增量 solve（#29）**：taffy 树跨帧持久 + 期望态 diff（style/measure context
  值比较短路、结构变更 set_children/remove，taffy 脏传播跳干净子树），替代每帧
  全量重建。api-infra 形状 release 实测：稳态 3.1ms / 单点变更 2.4ms vs 全重建
  29.8ms（~9.5×）。正确性由差分守卫测试保障（随机操作序列下增量 vs 全重建逐节点
  rect 全等）。
- **上帝文件拆分（#40，纯重构）**：`ffi/lib.rs` 4.4k 行 → 11 模块、
  `scene/control.rs` 4.8k → 9 文件、`list.rs` 3.5k → 12 文件。对外 API 与
  FFI 符号面零变化（143 extern 逐名对账）。

### Fixed
- **稳态帧文本误换行（发版后热修，#29 增量引入）**：增量 solve 的稳态帧 taffy 缓存全命中、
  measure 闭包不跑，text_layouts 每帧新建全空 → render 退回整数化宽度重测每个文本——
  宽度贴边的短文本被亚像素差误判换行（「首页/可堆叠/7天内发放/详细规则见」末字下行）。
  修后 text_layouts 承接上帧（同 measure_cache 模式），重测节点首测清槽恢复帧内语义；
  差分守卫测试加行数对比维度 + 稳态帧契约单测双保险。
- **#66 bounds 补偿幂等（review 批，blocker）**：MirrorPool 在 FULL 帧缓存 mesh
  原始 AABB，Header 帧（滚动中的旋转/缩放节点每帧都是 Header 级）从缓存重算——修前
  在已补偿值上再乘线性矩阵，scale<1 几何级缩小（#66 消失 bug 慢性复发）、90° 交替
  轴交换、45° 无界膨胀。新增两帧（FULL→HEADER×2）幂等回归测试（缩放/旋转两场景）。
- **文本导航字节映射方向（review 批）**：TextArea 上下行/行级 Home-End 的
  value↔display 偏移换算两参传反——掩码/IME 组装态（display 字节布局 ≠ value）错行
  错列；普通 ASCII 路径两向数值恒等故无感。掩码场景回归测试锁方向。
- **拖选门控（review 批）**：disabled 文本框不再响应拖选 Move（与
  on_pointer_down/occupies_gesture 对齐）；非主键 Down 不激活控件、不武装拖选/Slider
  跟随（浏览器对齐——右键按住拖动不扩展选区）。
- **EventBus once 语义（review 批）**：once handler 调 StopImmediatePropagation 后
  仍退订（修前 immediate-stop 的 break 在 once 收集之前，下次事件再触发一次）。
- **MaxLength 负值（review 批）**：C# setter 拒绝负数（FFI 参数 nuint，直接 cast 会
  把 -1 回绕成 ≈无限）。
- **letterbox fallback 数学（review 批）**：FFI 调用失败的 C# 兜底改与 Rust compute
  同式（top-down safe y + rendered span 双轴居中；修前用 Unity 下原点 y 且漏垂直
  居中项）。
- **take_warnings 内嵌 NUL（review 批）**：分条截断而非整串丢弃（任一条警告含 NUL
  曾静默吞掉全部）。
- **MirrorPool EditMode 测试升 v14（review 批）**：v10/v11 手搓 blob 自 frame blob
  v14（#26）起被 IsValid 拒收、整套必红；升 v14 列型 + node_id/parent_id 8B +
  ulong 反射键。

## [0.0.11] - 2026-08-25

v0.0.10 后三波累积：#48/#45/#43/#44/#46/#42 修复批、M2 分辨率适配批（#5/#3/#6/#7）、
狗粮批（#63-#67，Tripawd 实战打出来的五连修）。

### Added
- **分辨率适配（#5）**：Letterbox/FitWidth/FitHeight 三模式 + `vw/vh/vmin/vmax`
  视口单位（重排语言，随屏幕/适配模式重排）；`yio design` 命令 + GUI
  design/match 配置面。
- **Drag 事件载荷接线（#63）**：`DragMoveEvent.DeltaX/Y` 逐 Move 增量（core 权威，
  EventRecord 28B）、`DragStartEvent.StartPosition`、`Pointer{Down,Up}Event.Button`
  （web MouseEvent.button 值域，collector 读真右/中键）。语义定案见 public-api.md。
- **line-height px 形（#65 修复面）**：围栏拓宽 `<number> | <px> | normal`——
  px 按本元素字号换算，继承为 px（CSS computed 语义）。
- **交互原语路由指引（#67）**：视口平移 → `overflow:auto`（手势套件全自带、零
  拖拽数学）；Drag API → 对象拖拽低层积木。fence.md + editor/runtime skills 落位。

### Fixed
- **line-height px 形被当 27 倍 → 文本高度爆炸（#65）**：`line-height: 27px` 此前
  剥掉单位塞进倍数槽——17px 字号单行 459px、卡片溢出屏幕，且 `yio check` 不拦
  （Number 域不校验）。修后映射双槽 + `effective_line_height()` 换算 + 围栏值域
  门（em/% 打包期报错）。pkg 格式 v41→v42（旧 pkg 加载报 TooOld，需重打包）。
- **min-height:0 弹性滚动视口被内容撑爆（#64）**：overflow 容器（含装饰性
  `hidden`）的直接子此前被强制 `flex-shrink=0`——`.screen{hidden}` 的弹性链被
  锁死、预览↔运行时不一致。修后 shrink=0 只限真滚动容器（Auto/Scroll），并补
  CSS §4.5 specified-size 地板（显式尺寸子项不再被溢出行按比例挤扁）。
- **滚动容器内旋转节点消失（#66）**：非纯平移节点的 Unity renderer.bounds =
  GO 平移 × 未旋转 mesh ≠ 真实视觉 AABB → SRP 错误剔除。修后 Mesh.bounds 补偿
  为线性矩阵 × 顶点 AABB；MirrorPool dump 的 meshBounds 与实际一致。
- Issue #48/#45/#43/#44/#39 批：TabList 布局覆写、必需子 CSS 校验、Smooth
  滚动停错位、span 事件接线（详见下文）。

### Fixed
- **TabList 激活 panel 不再覆写作者布局（#48）**：激活 panel 此前被统一置
  inline `display:block`——作者写 `display:flex` 的 tabpanel 被改写，flex 行
  布局塌成纵向堆叠。现在激活分支清 inline display 回落作者 CSS（非激活保留
  `display:none` 剪枝），与浏览器 tab 库语义一致（JS 只管 ''/none，激活布局
  归作者样式表）。panel 显隐所有权归控件——作者不应在 panel 上写 display
  （showcase settings.html 的 4 处 `style="display:none"` 越权写法已清）。
- **ScrollToItem(Smooth) 平滑滚动停错位（#43）**：Smooth tween 的目标是一次性
  heights 快照——变高列表滚动中新可见项陆续测量、overlap 增长，tween 终点
  停在过期边界。现在 ScrollPane 持 `smooth_scroll_to` 锚，每帧 tick 在高度
  回填 + content_size 刷新后按最新 heights 重算 tween 终点；用户滚轮/拖拽/
  松手物理/编程 snap 接管时清锚。
- **span 级事件接线（#44）**：`hit_test_rich` 全链（core/FFI）此前就绪但零
  调用——点击 rich-text-block 内 span 命中容器、span 上的订阅永不触发。现在
  core `hit_subtree` 命中 rich 容器后细化到 run.source（span/TextNode/Image），
  事件产线天然带 span 目标，全部后端受益（main-design §10.2 事件归属契约
  兑现）；source 不可触摸/首帧无 layout 回落容器（HTML 语义）。

### Changed
- **fence 必需子节点 CSS 命中校验（#45）**：控件本体命中只证明作者在样式控件，
  不证明子部件被样式——thumb 无 background = 可拖不可见的隐形滑块头。现在按
  6.8 契约表对每个必需子实例查命中（option/listitem 多实例逐个查，template
  蓝图同查），任一无命中报 `FenceControlChildWithoutCss` error；combobox 补
  `data-slot=value` 必需子结构（漏写 = 选中值静默无显示）。fence.md §2.3/
  §6.7/§6.8/§7 同步。
- **双 CHANGELOG 漂移清理（#39）**：删根目录僵尸 CHANGELOG.md（2026-07-04 后
  未动、内容与树不符）；AGENTS 发版段指明唯一 CHANGELOG 在
  `unity/package/CHANGELOG.md`。

Issue #46/#42 批：box-shadow 层数围栏拦截、无滚动容器列表静默截断。

### Fixed
- **box-shadow 层数超限打包期报错（#46）**：渲染层合成 node_id 的 high-byte
  编码容量为 inset 8 层 / outer 4 层，超限层此前无任何拦截——第 9 层 inset
  的合成 id 撞 outer 编码区（层序错乱）、第 5 层 outer 落识别区外（shadow
  mask 不传播、C# 解码歧义），全部静默错渲染。现在 `parse_box_shadow` 超限
  整条拒收，fence 共享值域门（inline + `<style>` 规则双路径，单一真相源 =
  core 解析器）报 `FenceBadCssValue`；渲染 push 处对运行时 inline override
  注入的超限层兜底跳过。fence.md 视觉节同步（原「层数校验不在围栏内」注记
  作废）。
- **数据驱动 ListView 无滚动容器不再静默截断（#42）**：`ItemCount` 的列表
  若自身与祖先链都无 `overflow:auto/scroll` 容器，此前拿 (0,0) 假视口恒走
  冷启动——超过初始 slot 数（5）的列表静默只剩前几项、零诊断。现在退化
  全量渲染（原 m1-listview spec 语义：宁可全渲染，不可静默截断）+ 一次性
  运行时警告。附带：ul 被直接父容器 flex 纵向拉伸（`flex-grow>0` 主轴 /
  `align-items:stretch` 交叉轴默认值）同样钉死高度不能滚，enter 时警告
  （短列表拉伸无害，warning 不 Err）；自滚模式与无 pane 场景不误报。

### Added
- **运行时警告通道**：core `Scene::warnings` 缓冲（推送方 warn-once 去重）
  + FFI `yio_stage_take_warnings`（drain 语义，多条 `\n` 连接）+
  `YioHost.RuntimeWarning` 事件（引擎无关层不直接打日志）——Unity Driver
  订阅转 `Debug.LogWarning`，配错一眼可见（此前此类问题零诊断）。

## [0.0.10] - 2026-08-24

Issue #1/#2/#4 批：打包失败静默弃包、slot 投影行不参与宿主布局、F9 命中链探针。

### Fixed
- **bridge 错误不再静默吞掉（#1）**：悬空 slot 投影（页面投影 `slot="X"` 而组件
  模板无此槽）或展开域 id 撞车（投影 light 子 id 与组件模板 id 同名）此前让
  `yio build` 打印 OK、exit 0，**pkg.bin 悄悄不落盘**（旧文件先被清掉）——CI
  绿灯之下产物消失。根因：analyze 只消费诊断列表、丢弃只有 message 的 bridge
  失败。现在错误以 `PackError` Error 级诊断可见（build/check 都 exit 1 并指明
  出错页面）；失败但无 Error 诊断的路径由 analyze 兜底合成，此类吞错永不复发。
  OK 行现在带 package 数（`OK: 1 package(s), 2 atlas(es), 2 font(s)`），
  产物数量对 CI 可见。
- **slot 投影行按自身 display 参与宿主布局（#2）**：显式 `display:flex` 的
  span 此前在父容器的 rich-text 分类里被当 inline 子——父容器烙上
  rich-text-block 标记后，投影进该 span 的行元素整棵被折进一行 inline 流
  （「攻 13 防 7 堆一起」），div 行更被防御性跳过直接隐身。现在显式 flex 的
  span 在分类里算 block 子（浏览器 `display:flex` 外层块级）：父容器不再
  折叠，投影行进 flex 排版各占一行。新错误码 `FenceSlotInInlineContext`：
  `<slot>` 位于无显式 flex 的 span 内直接报错（inline 上下文里投影块级子
  无法按自身 display 布局；slot 放进 div 或给 span 显式 flex）。

### Added
- **F9 命中链调试探针（#4）**：编辑器/开发构建按 F9 开启——指针位置实时
  Pick，顶层命中变化时 Console 打印命中节点到根的祖先链（每层 HTML id /
  class / C# 类型 / opacity / touchable / world rect）。「看不见但接鼠标」
  的演出层偷命中时链顶即凶手（opacity=0 且 touchable=True）。本体
  `YioDebugProbe.DescribePickChain(ctx, x, y)` 常驻可用（正式构建自定义
  热键绑定）。配套：`Node.Id` 从数值占位换成真 HTML id 读取（新增
  `yio_stage_get_node_id_attr` / `yio_stage_get_node_classes` FFI）。

## [0.0.9] - 2026-08-23

### Fixed
- **rich 文本的空白折叠接入 CSS 语义（N25 定案）**：inline 容器里标签间空白
  文本节点（HTML 源码换行+缩进，如 `</span>
    <span>`）此前把 `
` 当独立
  词送进字形链——字体 cmap 不映射控制字符 → `.notdef` tofu 框（还占 .notdef
  advance 撑宽行）。战斗 tips「到处 tofu」的悬案即此：tips 是投影内容密集区，
  每条 tip 的 span 之间都有空白节点。现按浏览器语义折叠：`	`/`
`/`
`/
  换页与空格同为可折叠空白，纯空白节点折叠成单个空格 token（inline 兄弟间的
  源码换行渲染为一个空格），词内换行同样折叠。0.0.8 的缺字日志正是它点名
  `U+000A` 定的案——日志保留原样，继续作为 tofu 的第一取证通道。

## [0.0.8] - 2026-08-22

Tripawd Field Notes 三批（地图交互/演出打磨）回应：absolute 包含块浏览器语义、
挂载后布局就绪回调、缺字 tofu 取证日志、pkg 版本错配专属报错、演出 API 补口。

### Added
- **absolute 包含块 = 最近 positioned 祖先（N24，浏览器语义）**：声明
  `position: absolute` 且任一 inset 显式的元素，包含块取最近声明
  `relative`/`absolute` 的祖先（无则视口）——此前取直接父级，与浏览器分歧。
  `position: static` 进入围栏（显式回退初始值；schema 默认值同步修正，
  CSS 初始值本就是 static 而非 relative）。已知限制：inset 全 auto 的
  absolute 保持直接父静态位置；overflow 裁剪链仍随 DOM 祖先。pkg 格式
  v39→v40（旧 runtime 读新包 TooOld，重打包即迁移）。
- **缺字诊断日志（N25 取证）**：shaping 全链（主字体+回退）缺某字时，
  Console 点名 `font-family "X" has no glyph for 'c' (U+....)` + 修法
  （tofu 框本体不变——开发期故意暴露）。会话级去重（同字体族+字符只报
  一次），`YioHost.MissingGlyphReport` 事件暴露给引擎层。
- **`CallAfterLayout(cb)`（N26）**：tick 后 fire 的一次性回调——刚
  `Instantiate` 的子树在本回调里读 `Geometry` 已是实测值（`CallNextFrame`
  帧头 fire 先于 solve，新子树首读必全零）。业务免逐帧自旋等待。
- **`Play(name, durationSeconds)` 重载（N27）**：无 `animation:` 声明绑定的
  keyframes 无声明层时长，`Play(name)` 固定按 1s 播（无 delay/单次/normal/
  fill both/cubic-out，已随包文档写明）；重载让程序化演出节奏由调用方给。
- **pkg 版本错配专属报错**：Unity 包与 yio.exe 只升一侧时，
  `load_package` 报 `pkg format v38 is older than this runtime's v39 …
  re-run yio build with the matching yio.exe`——不再淹没在通用 malformed
  文案里（此前报错完全不提版本，只能靠经验定位）。

### Changed
- **`NodeStyle.TextColor`（N29）**：文字色内联通道此前叫 `YioColor`（类型名
  误入属性名，几乎不可发现），补 `TextColor`（与 `BackgroundColor` 对称），
  旧名保留为 Obsolete 别名（同一 "color" 通道，零 core 改动）。

## [0.0.7] - 2026-08-22

Tripawd Field Notes 二批（战斗手感）回应：transition transform 通道、组件死规则
警告、选择器报错细化、随包文档补伪类/显隐清单。

### Added
- **`transition: transform`（N18）**：transition 白名单扩到四通道
  （background-color / color / opacity / **transform**）。transform 按整矩阵
  TRS 分解插值（translate/scale/rotate 分量各自 lerp 后 SRT 合成，与 keyframe
  语义一致），镜像编码为负 y 缩放，x 轴坍缩退化不产 NaN；中途改向从进行中
  override 连续重锚（无 snap）。pkg 格式 v38→v39（旧 runtime 读新包显式
  TooNew，重打包即迁移）。box-shadow transition 仍不在白名单（多阴影列表
  插值语义复杂，roadmap 登记 defer）。
- **组件死规则警告 `FenceComponentRuleOutOfScope`（N22）**：组件 `<style>` 纯类
  规则的类名只出现在页面 host 外区域或其它组件投影内容上 → warning（组件 CSS
  不穿出 host，规则运行时恒死，浏览器预览却正常）。跨文件证据版——类名在组件
  模板/本组件投影内可命中、或全库不出现（运行时挂类）则静默，宁漏报不误报。

### Changed
- **选择器报错点名元凶（N18 连带）**：`unsupported selector` 从笼统整串不支持
  改为点名具体越界构造——未知伪类（`:not()`）、伪元素（`::before`）、通配
  `*`、组合子 `>`/`+`/`~`、高阶属性运算符（`^=` 等）各有专属文案。
- **随包文档**：css-reference 新增 Selectors 小节（伪类支持清单 + 越界构造
  清单 + transition 值域）；editor skill 临界规则补伪类一行（`:hover` 等每帧
  求值、无需运行时挂类）；runtime skill 补显隐官方通道
  （`node.Style.Display = DisplayMode.None`）。

## [0.0.6] - 2026-08-20

Tripawd dogfood Field Notes（N 系列）回应批：四个运行时 bug 修复、围栏值域门与
浏览器先验警告族、公共类型改名、工作区生成物刷新通道。

### Fixed
- **投影内容样式失效（N5/N6）**：组件 `<style>` 规则现在真正作用于 slot 投射的
  light 子（文档语义本就如此，运行时未兑现）。根因：投影 span 在页面宇宙被烘
  `rich_text_block` 折叠标志（页面侧分类看不到组件 CSS 的 `display:flex`），
  折叠优先于运行时 display。修复：rematch 应用 `display` 声明且终态为 Flex 时
  翻转布局策略（display 选择 Strategy 的架构不变量兑现）；`set_inline_override`
  同语义。span 带 class 规则 `display:flex` 也在打包期正确解除折叠。
- **flex 列居中容器内无宽文本逐字竖排（N7）**：taffy 某些测量轮次传
  `Definite(0)` 可用宽，首个 0 宽测量经 render 槽「Some 优先」策略钉死成多行
  布局。修复：退化 0 宽约束按无约束处理（浏览器语义：0 宽盒文本横向溢出）。
- **重复 `Node.Play` 静默无效（N8/N11）**：programmatic player 不回收——旧
  Completed+fill-both player 每帧续写末值且永不回收，player 无限累积。修复：
  `Play` 按「同节点+同名」替换旧 player，重复调用 = 确定性从头重播。
- **同节点换名 `Play` 被旧 player 遮蔽（战斗第二回合起动画不播）**：同名回收
  之外，不同名但动了相同通道（如都是 transform）的旧 player 仍每帧续写末值，
  新动画按 slotmap 槽序被静默盖掉。修复：`Play` 接管其所动通道——同名或通道
  重叠的旧 player（不限状态）一律回收；通道不相交（transform + opacity）仍
  共存可组合。
- **滑杆 thumb 偏上（N20）**：作者给 thumb 写定位（负 `top` 居中 / `left` 百分比）
  与控件自身的居中/位移 transform 叠加成双偏移。修复：thumb 定位权归控件——
  运行时逐帧归零其 inset/margin，位移全权由控件按 value 驱动；check 新增
  `FenceSliderThumbPositioned` 警告提示所有权（`left:0; top:0` 锚定与尺寸/外观
  不受影响）。附带修复：inset 四边的 `%` 值此前被运行时静默丢弃（fence 广告的
  `LengthPercentAuto` 语法未兑现），现在按含块百分比正确解析。
- **卡内悬停消失/闪烁（N23）**：C# 事件层曾对 Enter/Leave 统一走 capture→bubble
  祖先链路由，而 core 的 RollOver/RollOut 按悬停链差分逐节点发射（mouseenter/
  mouseleave 语义，本不冒泡）——「后代退链」的 Leave 被误投给祖先订阅，与
  enter/leave 驱动的抬升动画叠加成自激振荡。修复：Enter/Leave 只派发给事件
  目标节点自身，其余事件维持冒泡。
- **InputSystem-only 项目 F8 诊断每帧抛异常（N3）**：`YioStageDriver.Update`
  的 F8 轮询按 `ENABLE_INPUT_SYSTEM` / `ENABLE_LEGACY_INPUT_MANAGER` 分流。
- `background-size: stretch`（schema 广告的默认值）此前被 core 静默拒（仅认
  `100%`）；`resize` noop 声明此前误报 `FenceBadCssValue`。

### Added
- **ProgressBar.AnimateValue(target, durationSec = 0.4)**：演出缓动糖——fill 宽
  度走布局通道无 CSS 过渡（transition 只支持背景/文字/透明三通道），C# 投影层
  easeOut 插值。`Value` 动画期间读回目标（数据值），直接赋值显式获胜并取消动画；
  重定向从当前显示值平滑转向。
- **围栏值域门（error，双路径统一）**：命名色（`red` 等）与 `transparent`
  （color 之外）、`overflow: clip` 及拼错值、`filter: blur/drop-shadow`、
  `transform: skew/matrix` ——运行时恒无效的浏览器合法值全部打包期报错（此前
  静默吞掉，上线即坏）；`<style>` 规则的 Keyword 值域与行内同门。
- **浏览器先验警告族（warning）**：`display: inline` 语义偏差（按 flex 处理）；
  `transition` 属性域外（含 `all`）；rich-text inline flow 内 span 的死
  width/height；页面侧只可能命中投影内容的类规则（样式墙下恒死代码）。
- **工作区生成物刷新通道**：`yio scaffold` 现为生成物全刷新（三 skill +
  `.yio/` CLI 自拷贝 + `.yio/scaffold.version` 版本戳；config/workspace.json/
  源文件不碰，无 `--agent` 时按在场 agent 目录自动探测）；`yio check` 发现
  版本戳落后出 `StaleScaffold` 警告；GUI 打开工作区时探测并亮「更新工作区」
  按钮（一键 = yio scaffold 子进程）。消费端更新流：UPM 更新包（新双 exe 随
  包落地）→ 跑一次 scaffold 刷新。
- runtime API reference 补齐：值类型工厂（`Length.Px/Pct`）、`YioColor` 4 参
  构造、transition 支持矩阵、ProgressBar 值域（`[0, Max]`，Max 默认
  aria-valuemax=100）与 AnimateValue、`Image.Src` key 格式与验证手段、
  YioStageDriver 序列化字段速查（`_designSize` 默认 1080×1920 竖屏警示）。

### Changed
- **公共类型改名（breaking，消 CS0104 歧义）**：`Animation` → `AnimationHandle`、
  `Color` → `YioColor`、`Vector2` → `YioVector2`、`Rect` → `YioRect`、
  `KeyCode` → `YioKeyCode`（接线层同时 `using Yio; using UnityEngine;` 时
  每文件必撞歧义，N10）。C# 侧机械替换；FFI ABI 不变。
- `box-sizing` 错误引导文案修正为 content-box 事实（此前误称 border-box）；
  Keyword 值错误消息列出合法值域。

## [0.0.5] - 2026-08-19
### Added
- **runtime skill 自足**：新增 `yio-runtime` 的 `references/api-reference.md`（随 init 落工作区会话根，完整公共 API 查找表——对象层级、控件 role 全表、事件、ListView、动画、样式、异常）。此前 skill 把「完整 API 契约」指回 Yio 源码仓库的 `docs/design/public-api.md`，逼消费者 agent clone 源码翻文档；现以随包 C# 签名为准镜像成离线参考，skill 不再指路仓库。防漂移门加对账（role 宇宙 ↔ fence schema、skill 必须指名 references、禁止回指 repo 文档）。
- **yio CLI**（打包器 CLI 升格，二进制 yio.exe，随 Release 分发 + Editor/Tools 双 exe）：check（零写入校验，--format json 机读诊断）、build（结构化输出）、init（脚手架 + CLI 自拷贝到 .yio/ + 反向配置）、new / list / show / font add / atlas add（workspace 编排——AI 的主编辑路径）、version。
- 诊断 collect-all 修复：跨组件/跨包/注册表/资源（字体缺失、图集溢出、覆盖缺失与冲突）全量收集后统一报告，一次给全（修前首个含 Error 的组件即中断）；失败时 warning 一并携带。
- 退出码契约：0 干净 · 1 Error 级诊断/写命令冲突 · 2 用法/配置/io 错。
- 反向配置 `.yio/config.json`（ui_root + unity_root 双指针，基座）：output_dir 相对 Unity 工程根解析，AI 在会话根一步 yio build 直落 Assets/Bundles。
- 版本同轨：yio_pkg crate 版本 == Unity 包版本（release-check 断言），yio version 单一来源。
- **工作区拓扑重构（Tripawd 反馈：skill 困在 ui 目录、AI 会话管不到 Unity）**：会话根 ≠ ui 目录分离形态——skills 与 `.yio/`（yio.exe + config.json，整个入库，团队 clone 即得配套 CLI）落会话根，`yio.workspace.json` 留 ui 目录；`yio init <root> --ui <dir>`（省略 `--ui` = 单目录老形态），config 发现规则统一（会话根 / ui 本体 / ui 直接子目录都可作参数或 cwd）。不再生成 AGENTS.md / CLAUDE.md（入侵性），`--agent` 只决定 skills 目录（.claude/skills / .agents/skills）。
- **agent skills 三件全部重写**（对齐成熟 skill 范式：Figma figma-use / OpenAI figma-implement-design / unity-cli-loop）：`yio-editor`（操作手册——Critical Rules 集中开篇、增量工作流含浏览器预览自验步、❌/✅ 反模式、错误表带修法、收尾清单；完整查找表渐进披露到 references/ 三件）；`yio-runtime`（**新增**——场景挂载、加载管线钩子、`Get<T>`/事件、`IsPointerOnUI` 门控 3D、NativeHost 内嵌 3D、id 契约双面互指，补上「UI↔3D 桥」）；`yio`（uloop 范式命令手册 + workspace.json/config.json 字段表）。随包 `Editor/Resources/Yio/skill/` 副本删除（新拓扑下 root skills 全覆盖，消除三份漂移面）。
- **GUI 打包器向导双目录**：新建工作区选会话根 + UI 目录（默认 `ui`，允许 `.` 单目录形态）；「打开工作区」接受会话根或 ui 目录（config 发现解析），recent 列表存原始路径。
- `yio font add`：字体已在 `fonts/` 目录时跳过自拷贝直接注册（此前同源同目标拷贝在 Windows 报共享冲突，形似文件被锁）。
- **`<link rel="stylesheet">` 外部 CSS 支持**（Tripawd 反馈）：href 相对所在 HTML 文件（页面与组件同规则）、CSS 内 `url()` 相对 CSS 文件；规则/`@keyframes`/诊断与内联 `<style>` 同待遇，缺文件报 `FenceStylesheetNotFound`（此前静默丢弃）。
- 检查器修复（Tripawd 反馈）：class 规则声明的 `display:block` 此前不被 inline 上下文检查认（报错文案给的修法 (2) 失效），现与 inline style 同待遇；`FenceMixedInlineBlock` 文案不再误称 span 为 block container；组件 `@keyframes` 同名同内容多实例展开静默去重（此前每实例一条告警刷屏）；自定义元素嵌 `<span>` 的报错补教学（slot 属性写在直接子上）。
- 结构检查选择器覆盖对齐（Tripawd 反馈 12–14 批）：display 判定现认静态可判定的单 compound 选择器——class / id / 属性选择器（`[role="tablist"]`、`[data-slot="fill"]`），与控件 CSS 命中检查同覆盖；运行时可变状态属性（aria-checked 等）仍保守不放行。文档：装饰框（背景图 + 前景内容）canonical pattern 写进 skill 与 `FenceMixedInlineBlock` 文案；`switch`/`radio` 无框架槽位（knob 位移用 `[aria-checked]` 状态选择器）写进 skill 与 fence.md。

## [0.0.4] - 2026-08-18
### Added
- Runtime API：`z-index` 层叠、动画 longhand 属性、dropdown 视口定位（pkg 格式 v38）；调度器三件套、`UnloadPackage`、选项 getter、`GetTemplate`（pkg 格式 v37）。
- FFI 全导出统一 panic 边界（catch_unwind guard）；`get_live` 站点标签（函数名格式）+ 释放审计日志常驻，release dll 内「快照后死亡」类 panic 可一行定位。
- 围栏：控件结构 CSS 契约（combobox anchor + popup absolute 定位）；`-webkit-text-security`（disc/circle/square/none）。
- 打包器 GUI：per-project 最近列表 + 移除按钮；窗口标题改短为 "Yio"。
### Fixed
- `TextContent` 每帧重建泄漏 + NodeId 12-bit generation 回卷守卫；`TextContent` 清子后子 wrapper 正确标 disposed（调用方句柄读数抛 `ObjectDisposedException` 而非静默 no-op）。
- 世界空间控件几何 + 自滚动列表虚拟化；文本管线正确性、嵌套滚动命中、dropdown/滚轮手感（浏览器校准基线 + notch 单位直传）。
- pkg.bin 格式版本随 bincode layout 变更强制 bump（v35 旧包全部失效的根因修复，此后 v36→v38 逐版推进）。

## [0.0.3] - 2026-08-16
### Added
- showcase m2-animation #11/#12 程序化动画驱动接线：`Play` / `OnKey` / `OnHook` 锚点 + 动画句柄 Pause/Resume/Stop/Seek 全套演示。
### Changed
- 本版无 runtime 包变更（仅 showcase 演示接线与工程色彩空间设置）。

## [0.0.2] - 2026-08-16
### Added
- 组件系统：Custom Element 打包期展开（组件 registry + slot 接驳 + 完整校验集）、L3 查找作用域硬墙（`Get` / `Query` 不穿透组件 / List item 边界）、`custom_tag` 选择器与 `CustomElement.Tag`。
- 声明式动画：`@keyframes` / `animation`、`Container.Play` 句柄（Pause/Resume/Stop/Seek）、`Container.RestartAnimations` 原位重启。
- 视觉补全：CSS 渐变背景（linear / radial / ellipse + GRADIENT shader 变体）、box-shadow。
- 虚拟列表 slot 模型（parked-but-attached，slot 永驻子树、离场仅标记）。
- 打包器 GUI（Tauri）exe 闭环，Unity 菜单 `Yio > Open Packer` 拉起。
- runtime API 接线：`Node.Touchable`、`Container.ScrollPos`、NumberField 边界 / `Radio.Name` / Slider `IsIndeterminate`。
### Fixed
- 圆角裁切 SDF 像素空间化（rounded overflow clip 视觉扁平）；radial circle 关键字 extents、ellipse 角点 √2 贯穿。
- 文本叶子按可用宽度换行；显式 `min-width` 在被测量的文本叶子上保留。

## [0.0.1] - 2026-08-09
### Added
- 首个可安装 UPM 包。骨架链（div + 文字 + 图 + flex + cascade）从 HTML/CSS 一路通到 Unity 真机渲染。
- Runtime 公共 API 表面（Node/Container/Button/... 类型化投影层）。
- 围栏验证器（标准 HTML/CSS 子集，打包期报错）。
