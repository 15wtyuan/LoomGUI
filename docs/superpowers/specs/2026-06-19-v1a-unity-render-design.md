# LoomGUI v1a · Unity 渲染打通 设计（spec）

- 日期：2026-06-19
- 状态：待审 → 批准后进 writing-plans
- 依据：`docs/design/00-main-design.md`（v1 实现真相源）、`docs/roadmap/v1-scope.md`、`docs/superpowers/specs/2026-06-18-v0-skeleton-design.md`（v0 已落地）
- 参考实现：FairyGUI-unity（`temp/FairyGUI-unity/`，渲染/对象模型/批合/文本的机制参考）
- 产出方式：superpowers brainstorming 流程

---

## 1. 目的

v1 按"先打通 Unity 渲染（最薄竖切）"策略拆成 5 个子项目（见 §3）。**v1a 是第 1 个**：在 v0 纯 Rust 核心已端到端跑通的基础上，证明 **Rust → FFI → Unity 镜像 → 真渲染** 这条最大、最不确定、最被低估的缝（v1-scope §5）成立。

v1a 用 **v0 内存直通**（`load_html` dev-only，不含打包器），**不含事件/滚动/动画**。通了再投 v1b-v1e。

**v1 解的最大风险**：v0 已验"纯 Rust 核心能跑"。v1a 验"核心产出的渲染树跨 FFI 进 Unity、镜像成原生渲染对象、真画出来"——这是 v1 目的"证明架构成立"的核心一哆嗦。

---

## 2. 范围边界

**做**：
- csbindgen FFI：单块连续帧 blob（SOA 公共头 + mesh/text arena）+ 一次原子拷贝。
- Unity GameObject 镜像池 diff（NodeId→GO，O(n) 增删复用，Unchanged 跳过）。
- MaterialManager（DrawState 缓存，key 含 mask_context）+ URP unlit shader（Image/Text 共用）。
- rect mask：shader uniform `_ClipBox` discard。
- 根 Stage + UI 相机 + 参考分辨率缩放（MatchWidthOrHeight，fgui 两层压一层）+ 一次性 y-flip。
- MonoBehaviour 驱动（LateUpdate）+ Domain reload 保护（G13）。
- Rust→.dll 交叉编译 + csbindgen codegen 构建管线。
- 消费 v0 核心已产的 `sort_key`（FairyBatching 重排算法本身 v0 已有，v1a 不重写）。

**不做**（见 §7 defer 表）：
- 打包器 `loomgui_pkg` + 真纹理加载（PNG/GPU/TexId）→ v1b。v1a 图片用 1×1 白占位（显示为 tint 色块）。
- 事件/命中/拖拽/IME/`is_pointer_on_ui` → v1c。v1a 不接输入（`set_input` 传空）。
- GTween/ScrollPane/参考分辨率之外的响应式（safe-area）→ v1d。v1a 参考分辨率做（静态），safe-area 不做。
- 冷帧/换页帧 ≤2ms FFI 压测、FairyBatching 实机优化 → v1e。v1a 做静态 500 节点 stress（便宜帧）。
- shape mask/stencil（v1.x）、grayed（可选，shader 预留 keyword）、SRP Batcher/合 mesh（v1.x）。

**范围决策记录**（brainstorming 钉定）：
- **拆分策略**：选"先打通 Unity 渲染（最薄竖切）"而非 roadmap 原序（打包器先）。理由：最大风险先验，早出可见结果；打包器是低风险体力活，不解任何风险。
- **渲染管线 URP**（非 Built-in）：Unity 6.5 正式弃用 Built-in RP；目标=支持最新版 → URP 是唯一不背技术债的选择。URP unlit shader 比 Built-in 仅多一点 boilerplate。
- **图片占位 1×1 白**：v1a 只验渲染管线，真 PNG 留 v1b。
- **静态帧**：`tick(input=∅, dt=0)`，首帧直出渲染树。
- **文本全做**（测量在 v0 已完成，v1a 做后端光栅化+摆位）：文本是渲染管线的硬骨头，v1a 必须啃下（实现顺序放第 5，Image 先跑通管线再啃）。

---

## 3. v1 拆分上下文（v1a 是 #1）

| 子项目 | 范围 | 出口验收 |
|---|---|---|
| **v1a · Unity 渲染打通**（本 spec） | FFI+镜像池+shader+rect mask+根 Stage+Domain reload | v0 fixture 在 Unity 真渲染，500 节点静态无卡顿 |
| v1b · 打包器+资源管线 | loomgui_pkg+二进制包+图集+真纹理 | 从二进制包加载，散图/图集真显示 |
| v1c · 事件+交互状态 | 命中/click/hover/:hover:active/disabled/拖拽仲裁/is_pointer_on_ui/输入采集 | 按钮 hover/active 反馈，UI 挡游戏点击 |
| v1d · 滚动+动画+响应式 | GTween+ScrollPane+参考分辨率+safe-area | 可滚动容器，自适应分辨率 |
| v1e · 压测/打磨 | 冷帧/换页帧 FFI≤2ms、FairyBatching 实机、edge case | 性能基线全达标 |

依赖链：v1a 是地基；v1b/v1c/v1d 都挂在已通的 Unity 后端上，彼此相对独立。

---

## 4. 设计

### 4.1 FFI 边界（v1a 命门）

**每帧数据流（静态帧）**
```
Unity LateUpdate:
  [一次性] stage = stage_new(); stage_load_html(html, css)    ← v0 内存直通；v1b 换 load_package
  每帧:
    stage_tick(stage, input=∅, dt=0)               ← 静态
    view = stage_borrow_frame(stage)                ← Rust 拥有的帧 blob (ptr,len)
    Marshal.Copy view → ArrayPool buffer             ← 一次原子拷贝（§14.3），之后只读自身拷贝
    解析 blob → diff GO 镜像池 → 上传 mesh / 生成 text mesh → 设 transform/sort/clip
  // Rust 下帧 tick 开头 reset arena（零分配复用）
```

**最小 ABI**（csbindgen / `extern "C"` / context 指针模式，IL2CPP 友好）：
- `stage_new() -> *mut Stage` / `stage_free(*mut Stage)` — opaque handle（C# 持 `IntPtr`）
- `stage_load_html(stage, html_utf8, css_utf8)` — **v1a dev-only 内存直通**，v1b 换 `load_package(bytes)`
- `stage_tick(stage, input_ptr, input_len, dt)` — v1a 传空 input（事件 ABI v1c 定）
- `stage_borrow_frame(stage) -> { ptr, len }` — Rust 拥有，下帧 tick 前有效

**帧 blob 布局**（§14.3 SOA + arena，做成一块连续）：
```
[ frame_header 元信息：各列/各 arena 的 offset+len ]
[ 公共头列（并行 SOA）：node_ids | parent_ids | visible | alpha | grayed | color_tints
                       | transforms | blends | mask_contexts | sort_keys
                       | payload_kinds | payload_arena_idx | payload_offsets | payload_lens ]
[ mesh_arena：verts[f32] | uvs[f32] | colors[u32] | indices[u16] ]
[ text_arena：glyphs_soa | runs_soa | lines_soa ]    ← §9.2 三表
```
- 每节点 = 公共头各列第 i 项；payload 由 `(payload_kind, arena_idx, offset, len)` 三元组定位。
- `Unchanged` 节点三元组空，不占 arena。
- C# 用 `Span<byte>` + `BinaryPrimitives` 读，**禁用 `Marshal.PtrToStructure`**（IL2CPP 对齐坑）。

**所有权**：Rust 拥有 per-frame blob，下帧 tick 开头 reset（零分配复用）。C# borrow 后立即 `Marshal.Copy` 到 `ArrayPool<byte>.Shared.Rent(len)`（v1a 先 `new byte[]` 也行，ArrayPool + 冷帧 ≤2ms 留 v1e）。**绝不跨 FFI 长期持裸指针。**

> **取舍**：帧数据走单块连续 blob + 一次拷贝（非多段指针分别拷）——C# 侧最简，贴合 §14.3"原子拷贝"语义。

### 4.2 Unity 后端三件套

**(a) GameObject 镜像池 diff**（G9 / §14.6，照 fgui `NGraphics` 对象模型）
- 池 `Dictionary<uint nodeId, RenderObj>`，RenderObj = `GameObject{MeshFilter+MeshRenderer}` + Mesh（`MarkDynamic()`）+ scratch。
- 每帧三步：①池中全标 stale → ②遍历 frame blob 节点，按 node_id 查池：命中→清 stale + 按 payload 更新（`Unchanged` 跳过）；未命中→新建 GO → ③余下 stale 销毁。**O(n)**。
- Container 类型节点产空 GO（不挂 MeshRenderer），叶子节点产 GO+MeshFilter+MeshRenderer（照 fgui Container 无 graphics）。
- **绘制序**：`meshRenderer.sortingOrder = sort_key`（照 fgui `NGraphics.cs:386`，`renderingOrder` 每帧重置；核心 sort_key 即此计数器）。UI 平铺 z=0，不用 z 排序。
- 根 Stage GameObject `localScale=(sf,-sf,sf)` 一次性 y-flip + 参考分辨率缩放合一（见 4.3d）。
- rect mask **不建独立 GO**（G9「Mask 独立对象」指 v1.x shape mask 的 stencil write/erase；v1a rect mask 走 shader uniform）。

**(b) MaterialManager（DrawState 缓存，§8.4，照 fgui `MaterialManager.cs`）**
- key = `(program, flags, blend, texture, mask_context)`——**mask_context 必须进 key**（rect mask 的 `_ClipBox` uniform 每个 clip 上下文不同，不同 context 必须是不同 Material 实例，否则 clip uniform 冲突）。照 fgui `(flags, blendMode, group=clipId)`。
- v1a：program = Image/Text，flags 可含 Grayed（可选），blend = Normal，mask_context = clip 上下文 id。
- **不用 MaterialPropertyBlock**：tint×alpha **烤进顶点色 `Color32.a`**（照 fgui `UpdateMeshNow:764-788`，shader `col = tex2D * v.color`）；clip_box 进 mask_context 专属 Material；blend 走 material property（SrcFactor/DstFactor，非 shader variant，省 variant 爆炸）。
- 图片 v1a：所有占位 tex_id → 同一张 1×1 白贴图（图片显示为 tint 色块）。

**(c) URP unlit shader**（Image/Text 共用，照 fgui `FairyGUI-Image.shader` URP 化）
- `col = tex2D(_MainTex, uv) * v.color`、`Cull Off`（根 y-flip 使 winding 反转，必修）、`ZWrite Off`、queue Transparent、`Blend [_Src][_Dst]` property。
- variant：`#pragma multi_compile _ CLIPPED` —— CLIPPED 分支做 `_ClipBox` discard（`col.a *= step(max(|clipPos|),1)`，clipPos 在 vert 由世界 xy 算）。
- URP 移植：`UnityCG.cginc`→`URP Core.hlsl`、`UnityObjectToClipPos`→`TransformObjectToHClip`、删 `Fog{}`、SubShader 加 `UniversalPipeline` tag。v1a 砍掉 SOFT_CLIPPED/COMBINED/COLOR_FILTER/ALPHA_MASK，只留 CLIPPED。
- grayed keyword 预留（v1a 可选）。

**(d) Text（Unity 只当光栅化器，§9 契约）**
- 借鉴 fgui `DynamicFont`：`Font.RequestCharactersInTexture` + `GetCharacterInfo` 取 glyph UV + 像素边界（bbox）。
- **改**（LoomGUI 对 fgui 的有意改进）：丢弃 Unity `CharacterInfo.advance` + `fontSize*1.25` line-height（§9.1 明令禁止），位置**严格按 Rust TextLayout 绝对坐标**（glyph.x + bearing）。fgui 信了 Unity advance → 跨平台微差；LoomGUI 用 ttf-parser 真度量解决它。
- **坑（必修）**：Unity 动态字体 atlas 异步 rebuild 时 **glyph UV 会变**（advance 一般不变）→ 必须 `Font.textureRebuilt` 回调 + version 计数，变了重取 UV（照 fgui `DynamicFont.cs:356-375`）。不抄会画错字。
- v1a 先单字体（DejaVu，ASCII），CJK 字体策略留 v1b。

### 4.3 工程搭建 + 构建管线

**(a) 构建：Rust→.dll + csbindgen**
- Rust 端加 cbindgen 导出层，§4.1 最小 ABI 全 `#[no_mangle] extern "C"`，string 走 UTF-8 `byte*`，handle 走 opaque `*mut Stage`。
- csbindgen 生成 C# `[DllImport]` → 落 `loomgui_unity/Assets/Plugins/LoomGUI/Bindings/*.cs`（**gitignore 忽略、不入库**，构建时生成）。
- 交叉编译 v1a 只 **Win x86_64 MSVC → `.dll`** → 拷 `Assets/Plugins/LoomGUI/loomgui_core.dll`（**入库**，`!**/Plugins/**/*.dll` 白名单覆盖）。Mac/IL2CPP 留后。
- `.dll` 的 `.meta`：Platform=Editor+Standalone Windows, CPU=x86_64。

**(b) Unity 工程结构**
- `Assets/Plugins/LoomGUI/`：native `.dll` + csbindgen Bindings（asmdef `LoomGUI.Bindings`，开 unsafe）。
- `Assets/LoomGUI/`：后端 C#（Stage Behaviour / 镜像池 / MaterialManager）asmdef `LoomGUI.Runtime`，引用 Bindings。
- `Assets/LoomGUI/Shaders/LoomGUI-Unlit.shader`：§4.2c。

**(c) Stage MonoBehaviour 驱动**（照 fgui `StageEngine.cs:20`）
- `[ExecuteAlways]`（Edit mode 也跑，方便预览）+ `useGUILayout=false`。
- `LateUpdate()`：`set_input(空)` → `tick(dt)` → `borrow_frame` → `Marshal.Copy` → diff 镜像池 → 同步。
- 一次性 `Awake`：`stage_new()` + `load_html(...)`。

**(d) 根 Stage + 相机 + 缩放**（fgui 两层压一层，照 §7.4）
- UI 相机：正交、`cullingMask=1<<6`（LoomUI layer，已配）、`clearFlags=Depth`、关 HDR/MSAA（已配）；URP 挂 `UniversalAdditionalCameraData`。
- 根 Stage GameObject：`localScale=(sf,-sf,sf)`（scaleFactor `sf=min(screenW/designW, screenH/designH)`，MatchWidthOrHeight 照搬 fgui `UIContentScaler`；与 y-flip 合一）；相机 orthoSize=screenH/2（世界=像素）；屏幕变 → 重算 + 触发 Rust solve。精确 transform 数学留 plan。

**(e) Domain reload 保护（G13，必修）**（照 fgui `Stage.cs:86`）
- `[RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]` → 调 Rust `loomgui_shutdown()` + 清 C# 镜像池/MaterialManager 缓存。
- 必要性：禁 Domain Reload 下 C# statics 不清零但 native 句柄已释放 → 野指针 crash。Rust-native 库必修课。

### 4.4 实现顺序（风险隔离）

1. 工具链 round-trip（`version()` 过 FFI 来回）→ 验 Rust→.dll+csbindgen 通。
2. 单 quad（FFI blob + 镜像池 + URP shader 跑通一张图）→ 验渲染管线最小闭环。
3. 多节点 + `sort_key`/`sortingOrder` → 验绘制序。
4. rect mask（`_ClipBox`）→ 验裁剪。
5. **Text**（光栅化 + `Font.textureRebuild` 监听）→ 最高风险放后。
6. 500 节点静态压测。
7. Domain reload 保护。

---

## 5. fgui 对照表（参考实现映射，照"实现前先参考 fgui"准则）

| 机制 | fgui 实现 | LoomGUI v1a | 处置 |
|---|---|---|---|
| 绘制序 | `meshRenderer.sortingOrder = renderingOrder++`（每帧重置） | `sortingOrder = sort_key` | 照搬 |
| FairyBatching | 稳定插入排序+AABB，**只重排不合并 mesh** | v0 核心已实现，v1a 消费 sort_key | 照搬（不合并 mesh 已对齐） |
| 对象模型 | DisplayObject 1:1 GO，Container 无 graphics | NodeId→GO 镜像，Container 空 GO | 照搬 |
| Material 缓存 | `(flags, blend, group=clipId)` | `(program, flags, blend, texture, mask_context)` | 照搬（mask_context 进 key） |
| Tint/alpha | 烤进顶点色 Color32.a，不用 MPB | 同 | 照搬 |
| Blend | material property（Src/Dst factor）非 variant | 同 | 照搬 |
| rect mask | shader `_ClipBox` discard（非 stencil/scissor） | 同（URP CLIPPED variant） | 照搬 |
| shape mask | stencil write/content/erase 三态 | — | v1.x |
| y-flip | **逐节点 `y=-y` SetPosition** | 根 `(sf,-sf,sf)` 一次翻转 | **LoomGUI 自选（更优）**；shader 须 Cull Off |
| 参考分辨率 | 两层（unitsPerPixel@Stage + scaleFactor@GRoot） | 一层（世界=像素 + 根 scale=scaleFactor） | 压层简化；MatchWidthOrHeight 照搬 |
| 驱动 | LateUpdate MonoBehaviour | 同 | 照搬 |
| Domain reload | `[RuntimeInitializeOnLoadMethod(SubsystemRegistration)]` | 同（调 Rust shutdown） | 照搬（必修） |
| Text 测量 | 自己断行 + 信 Unity `CharacterInfo.advance` + `fontSize*1.25` | Rust ttf-parser 真度量，**丢弃 Unity advance** | **LoomGUI 改进**（§9.1） |
| Text 光栅化 | `RequestCharactersInTexture`+`GetCharacterInfo` 取 UV/bbox | 同 | 照搬 |
| Text atlas rebuild | `Font.textureRebuilt` + version 计数 | 同 | 照搬（必修坑） |

---

## 6. 主文档已改记录（本 spec 引入的订正）

写本 spec 时对照 fgui 源码，修正 `docs/design/00-main-design.md` 两处自相矛盾/误读：
- **§14:625**：「rect 遮罩用 stencil」→「shader uniform `_ClipBox` discard（§8.6；shape mask 才用 stencil）」。原 §8.6:360 写对了，§14 笔误，现统一。
- **§8:1**：「Unity 根 GameObject 挂 (1,-1,1) scale（fgui 同款）」→「LoomGUI 自选；比 fgui 逐节点 `y=-y` 取负更干净——只翻一次；副作用 winding 反转 → Unity shader 须 `Cull Off`」。fgui 实为逐节点翻，非根翻转。

---

## 7. defer → 落地表

| defer 项 | 落地阶段 | 依据 |
|---|---|---|
| 打包器 `loomgui_pkg` + 二进制包 + 图集 + refcount | v1b（G1） | v1-scope §3 G1 |
| 真纹理加载（PNG/GPU/TexId 注册） + 字体资源完整化 | v1b（G7/G6） | v1-scope §3 |
| 事件/命中/拖拽仲裁/IME/`is_pointer_on_ui` + 输入采集 | v1c（G4/G5） | v1-scope §3 |
| GTween + ScrollPane（惯性/回弹/滚动条）+ safe-area | v1d（G12/G14） | v1-scope §3 |
| 冷帧/换页帧 FFI ≤2ms（ArrayPool）+ FairyBatching 实机优化 | v1e | v1-scope §4 |
| shape mask/stencil、grayed（可选预留）、SRP Batcher/合 mesh、scale9/tile/fill、ColorFilter、soft clip | v1.x | v1x-deferred.md |

每条有归属，不悬空。

---

## 8. 风险（高→低）

1. **FFI SOA+arena 布局 + C# Span 解析**——新东西，跨边界字节序/对齐易错。缓解：先跑通单节点（1 quad）。
2. **csbindgen + Rust→.dll 工具链**（首次搭建）。缓解：先 `version()` round-trip。
3. **Text 光栅化 + atlas rebuild 监听**（§4.2d）。缓解：Image 先跑通，Text 第五阶段。
4. **Domain reload 野指针**——必须做 §4.3e 保护。
5. **URP shader port**（Cull Off/clip discard/blend property）。缓解：照 fgui Image.shader URP 化。
6. **clipBox 坐标空间**——fgui 用世界空间，核心传世界空间 clip_rect，后端算 clipBox。缓解：照 fgui `UpdateContext.cs:105-156` 公式。

---

## 9. 验收标准（"v1a 跑通"）

1. v0 的 fixture 面板（div flex + 文本 + img + rect mask）在 Unity Play/Editor **真渲染**出来：布局/文本/图片/裁剪都对。
2. `version()` 过 FFI round-trip；单 quad→多节点→rect mask→Text 逐级跑通（实现顺序 §4.4）。
3. **500 节点静态 UI 每帧无卡顿**（v1 中段 stress，便宜帧；冷帧/换页帧留 v1e）。
4. 进/出 Play Mode 不 crash（Domain reload 保护生效）。
5. native `.dll` 入库、csbindgen 绑定不入库（构建生成）。

---

## 10. 工期估算（参考）

~3-5 周（v1a 是 v1 最大单块之一，含首次工具链搭建税）。分阶段：
- 工具链 round-trip + 单 quad：~1 周
- 多节点 + sort_key + rect mask：~1 周
- Text（光栅化 + atlas rebuild）：~1 周（最高风险）
- 500 节点压测 + Domain reload + 打磨：~0.5-1 周
