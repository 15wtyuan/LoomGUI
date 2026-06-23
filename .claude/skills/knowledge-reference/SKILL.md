---
name: knowledge-reference
description: >
  Use when working on LoomGUI (loomgui_core Rust 核心、HTML/CSS DSL、taffy flexbox、
  ttf-parser 文本测量、RenderNode 渲染树、FFI、Unity/Godot 后端), researching LoomGUI
  architecture/mechanisms/design decisions, hitting taffy 0.5 / ttf-parser 0.20 / cssparser 0.34
  API issues, looking up pitfalls/debug techniques/known issues, or needing project implementation
  context before starting work. 项目实操知识库（踩坑/API/调试/机制实现），随开发累积。
---

# LoomGUI 知识参考

LoomGUI 项目实操知识库：架构索引、各层机制、依赖 API 适配踩坑、AI 可预测性约束、踩坑记录、调试技巧、已知问题。

> **★ 工作准则（牢记）：动任何机制前先对照 fgui 源码。** LoomGUI 的渲染/对象模型/批合/事件/动画/资源管线全面借鉴 FairyGUI（参考实现 `temp/FairyGUI-unity/`）。**实现任何功能前**先 grep/读 fgui 对应文件看它怎么做的，再定 LoomGUI 设计——避免走歪。本 session 因没先看 fgui 的 sortingOrder/rect-mask/MaterialManager，初版设计走了弯路（误用 z 排序、误以为 rect mask 要独立 GO、绘制序想复杂）。对照时注意 fgui 是 Built-in RP（URP/shader/材质 API 要适配，见 §3.5/3.6）。

## 0. 本文件 vs docs 分工（先读这个）

| 文档 | 性质 | 何时读 |
|---|---|---|
| `docs/design/00-main-design.md` | **设计权威契约**（v1 真相源，710 行） | 查设计意图/契约（§4 围栏 §5 parse §6 Node §7 taffy §8 render §9 text） |
| `docs/roadmap/v1-scope.md` | 范围 | 查 v1 干什么/围栏冻结/胶水任务 |
| `docs/roadmap/v1x-deferred.md` | defer 草稿 | 查 v1.x/v2 机制 |
| `docs/review/` | 五轮对抗审查归档 | 追溯决策来源 |
| **本 skill** | **实操知识库** | 查「怎么干 + 坑在哪 + API 怎么用」 |

开工先读本 skill 知道实操上下文，设计查 docs/design。两者互补不重复。

## 1. 架构

LoomGUI = 跨引擎游戏 UI 框架（对标 FairyGUI）。**Rust 核心（loomgui_core，引擎无关）+ 多引擎后端（Unity 首发）**。

**核心动机**：AI 驱动界面拼装——HTML 作 DSL，让 AI 既能编辑（文本）又能预测渲染（AI 对 HTML/CSS 强先验）。**AI 可预测性是 DSL 决策的首要准则**（见 §4），背离浏览器语义的 divergence 须谨慎评估。

### 1.1 workspace（主文档 §16）
```
loomgui/                      # workspace
├── loomgui_core/             # lib，引擎无关（v0 已实现，砍 event/anim）
│   ├── src/{parse,style,layout,scene,render,text}/ + stage.rs
│   ├── examples/v0_snapshot.rs
│   └── tests/{snapshot.rs, fixtures/DejaVuSans.ttf, snapshots/}
├── loomgui_pkg/              # 打包器（v1 第一阶段）
├── loomgui_ffi_c/            # C ABI + csbindgen（v1a Phase 1 ✅ 已实现）
├── loomgui_unity/            # Unity 6.5 URP 后端（v1a Phase 1 ✅ 已实现）
└── loomgui_editor/           # 编辑器（v2+）
```

### 1.2 数据流（单向无环）
```
HTML/CSS → parse(ElementTree/StyleSheet) → style(ResolvedStyle) → scene(Node 树)
        → text(measure → TextLayout) → layout(taffy solve → layout_rect)
        → render(Vec<RenderNode>) → stage.tick → render_nodes JSON
```

### 1.3 渲染树契约（主文档 §8）
`RenderNode` = 公共头 + payload enum（`Unchanged`/`Mesh`/`Text`）。描述**渲染意图**，不规定引擎机制（后端自选 stencil/Material/canvas_item）。
- glyph 绝对坐标（§9.2）：核心已累加 advance + align 偏移，后端拼 quad 零累加。
- 纹理加载是**后端职责**，核心只持 TexId + UV。

## 2. 各层机制要点

### 2.1 parse（§5）
- `scraper` HTML + `cssparser` CSS + 自写 ~100 行选择器匹配器（不用 selectors crate，围栏窄）。
- `ElementData { tag, classes, id, text, attrs: HashMap<String,String>, children, parent }`。
- 行内混排（文本+元素同在）解析期报错。
- `match_element` 返回 specificity **降序**（元组 `(id数, class数, tag数)`）—— 下游 cascade 注意排序方向（坑 6）。

### 2.2 style（§5）
- `ResolvedStyle` = `taffy_style` + 视觉字段（bg-color/border/opacity/color/font-*/text-align...）+ `order: i32`。
- `resolve_styles` 自顶向下递归；继承白名单 8 字段（color/font-size/font-family/font-weight/line-height/letter-spacing/text-align/white-space）从父继承。
- `apply_decl`：CSS 声明 → taffy/视觉字段，**无条件覆盖默认**（时序：default → apply_decl）。
- **默认 flex-direction = Column**（见 §4 约束 1）。

### 2.3 scene（§6）
- `Node { id, parent, kind, style, taffy_id, layout_rect, clip_rect, children, dirty_mesh, dirty_text }`。
- `NodeKind`: `Container` / `Button` / `Image{src}` / `Text{content}`。
- div/button 裸文本 → Text 子节点（§4.2，文本是 flex item）。
- `overflow:hidden` → `clip_rect = Some`（layout solve 后填实际框）。

### 2.4 text（§9）
- ttf-parser 度量 + **unicode-linebreak UAX#14 断行（v1b.5 落地，替换 v0 `split(' ')` 贪心——后者对 CJK 完全失效：中文无空格→整段一 word 无法换行）** → `TextLayout` SOA 三表（lines/runs/glyphs）。greedy fill on break opportunities：CJK 逐字可断、ASCII 按词、`\n` mandatory、nowrap 单行、超长词（segment 宽>max_w 且多字）逐字断（参考 fgui `toMoveChars=1`）。
- `Glyph { glyph_id, codepoint, x, y, bearing_x, bearing_y }` 绝对坐标。**codepoint**（v1a Phase 2 加）供引擎字体 API（Unity `GetCharacterInfo(char)` 按码点非 glyph_id）。CJK codepoint ≤U+FFFF BMP，现有 u32 装得下，`GetCharacterInfo((char)cp)` 正常。
- v0 砍 rustybuzz/BiDi/fallback，仅 ASCII+CJK。**v1b.5 加 CJK 字体 fixture**（`tests/fixtures/wqy-microhei.ttc` 文泉驿微米黑 ~5MB，`Face::parse(bytes,0)` 取 .ttc index 0=Regular）+ `test_font_cjk()` skip-if-missing helper。defer：emoji/组合符号 shaping/RTL/kinsoku 标点禁则/font fallback 链/多 font-family/per-glyph font_id。

### 2.5 layout（§7）
- taffy 0.5 集成（**API 见 §3.1，与草稿差异大**）。
- MeasureFunc：文本调 `measure_text`；**Image 三档（v1b.2）**：CSS `Dimension::Length` > registry 真实 w/h > 64×64 兜底。`solve(scene,font,root,&TextureRegistry)` 4 参。
- `solve` 就地写 `layout_rect`（绝对坐标，父 origin 累加）+ `clip_rect`。
- **measure 陷阱（v1b.2 测设计）**：`solve` 把**根节点** taffy size 强制覆盖为 root_size（`set_style`）→ Image 作根时 intrinsic 被 viewport 覆盖（测须包 Container 根、Image 作子叶）；默认 `align-items:Stretch`（column 容器）会把无显式宽子项 cross 轴拉伸 → 测无 CSS 尺寸的 Image 须设 `align_self:FlexStart` 禁 stretch。

### 2.6 render（§8）
- `build_render_nodes(scene, font, &TextureRegistry)`：Container/Button→Mesh quad(背景色，全图 UV `[0,0],[1,1]`)，Image→Mesh quad（**tex_id 查注册表 + UV region**：v1b.3 按 atlas 子区 `uv_min/uv_max` 烤 4 角 UV，未注册=0 哨兵+全图 UV→白占位；v1b.2 前 v0 占位是 hash(src) 已删），Text→TextLayout 装 Text payload。`mesh::quad(rect,color,uv_min,uv_max)` 接 uv_rect（vert TL,TR,BR,BL ↔ sprite 角；v1b.2 全图即 0,0,1,1）。
- `assign_sort_keys`：DFS 单计数器 sort_key，clip 的 Container 是 BatchingRoot 开新 mask_context。
- **v1b.4 AABB 保序重排 + mesh 合并**（§8.5）：`build_render_nodes` 末尾 `assign_sort_keys → reorder_for_batching → merge_meshes`。`reorder_for_batching`（batch.rs）= fgui `DoFairyBatching`（Container.cs:877-941）稳定插入排序 core 化——同 DrawState((texture,program,mask_context)) 不相交元素前移聚拢，相交保相对序（坑 23）；Text(program=1) batch break 不重排。`merge_meshes`（merge.rs）按 sort_key 扫连续同 DrawState Mesh→拼 merged payload。**锚 node_id**（merged=min batch，坑 24）解动画 GO 抖动；**merged transform=0/alpha=1** 让 blob.rs:70 re-base（减 0）+ blob.rs:90 alpha 烤（×1）对 merged 无效 → **blob/MirrorPool 零改**（§2.8 列结构不动，spec §9）；colors 只烤 alpha 分量（rgb 不动，color_tint 不传，坑 9）。

### 2.7 stage
- `Stage::new(font_path, root_size)` → `load_inline(html, css)` → `tick_and_render()` → `FrameData{nodes:Vec<RenderNode>, clips:Vec<ClipEntry>}`（v1a Phase 2：clips=嵌套交集后的 clip 表）→ `render_json()`。
- 静态首帧：tick 接空输入、dt=0。

### 2.8 FFI（loomgui_ffi_c，主文档 §14，v1a Phase 1）
- `extern "C"` 薄包装 + opaque `*mut StageHandle`；csbindgen 扫 `src/lib.rs` 生成 C# `Native` 类。
- ABI：`stage_new/free/load_html/tick/borrow_frame/shutdown`。string 走 UTF-8 `*const u8`+len；`borrow_frame(h, *mut usize) -> *const u8` 返 Rust 拥有的帧 blob（下 tick 失效；未 tick 返 null+len=0）。
- `StageHandle{ stage, frame_blob: Vec<u8> }`——tick 时 `build_blob` 覆写 frame_blob。
- `build_blob(&FrameData) -> Vec<u8>`（**v3**, version=3，v1b.2）：SOA 公共头 **14 列**（v2 的 13 + `tex_id:u32` 末列——Image→真 tex_id，其余=0）+ mesh arena + **text_arena**（per text 节点 `font_size:u32|color:f32×4|glyph_count:u32|glyphs[{codepoint,pen_x,pen_y}]`）+ **clip 表**（`context_id→design rect`，嵌套交集）。`num_col_offsets=columns.len()` 自动传播列数→header_len=92。magic+version 进 header，C# `FrameBlob.IsValid` 校验（防 stale v2 blob）。**mesh 顶点 re-base 到节点本地**（减 transform.x/y）。全 LE。改 blob 格式必重编+换 .dll（坑 10）+ C# fixture 同步（坑 17）。
- v1b.3 FFI（常驻不 gate）：**删** v1b.2 的 `register_texture`/`image_src_count`/`image_src_at`（loose 散图模型被 atlas 取代）；**加** `atlas_count(h)->usize` / `atlas_info(h,i,*out_tex_id,*out_w,*out_h,*out_src_len)->*const u8`（返 atlas filename 无尾 NUL 串 + len，`*out_tex_id=(i+1)`，坑 16 len-based 读）。version 串 v1b.3。

### 2.9 Unity 后端（loomgui_unity，主文档 §14，v1a Phase 1）
- `FrameBlob`（BitConverter 解析 v2 blob，`IsValid` 校验 magic+version）→ `MirrorPool.Sync`（`Dictionary<uint,RenderObj>` O(n) stale-flag diff）。**flatten（Phase 2）**：所有 GO 挂**根**（非巢状——local_x/local_y 是绝对 design 坐标，巢状 SetParent 会双计父位置，坑见 §2.11/Phase 1 单节点未暴露），`localPosition=绝对`、`sortingOrder=sort_key`；kind=1 Mesh / kind=2 Text（→TextRasterizer）/ kind=0 跳过。**buffer 复用**：RenderObj 持可复用 List，`SetVertices(List)` 零 alloc（T7，500 节点压测）。
- `MaterialManager`：key=(program, texture, mask_context)——mask_context 进 key → 每 ctx 独立 Material 持各自 `_ClipBox`；ctx>0 → `EnableKeyword("CLIPPED")`（`#pragma multi_compile`）+ `SetClipBox`。**tint×alpha baked 进顶点色（Rust 侧）**，材质只带 texture+clip_box+blend。
- `LoomStage`（`[ExecuteAlways]` MonoBehaviour）：LateUpdate `tick→borrow_frame→Marshal.Copy→FrameBlob→MirrorPool.Sync`。根 GO `localScale=(sf,-sf,sf)`（shrink-to-fit sf=min(sw/dw,sh/dh) + y-flip 合一）+ `localPosition=(-sw/2,sh/2,0)`；UI 相机正交 `orthoSize=sh/2` `cullingMask=1<<6`(LoomUI) **独立于根**（不 SetParent）。shader `Cull Off`（根翻转 winding）。Phase 2：`[SerializeField] Font _font`（EnsureFont 兜底 AssetDatabase 加载 DejaVu）、`Font.textureRebuilt+=OnRebuilt`（OnDestroy 解绑）、`ResetStatics`（`SubsystemRegistration` 调 `loomgui_shutdown`+`TextRasterizer.ResetStatic`）、**Awake 清 root 下 loom_node 孤儿 GO**（ExecuteAlways 防累积，坑 11）。
- URP unlit shader：`col=tex2D×v.color`、`Cull Off`、`ZWrite Off`、`Blend[_Src][_Dst]` property、`CLIPPED` variant（rect mask `_ClipBox` discard，Phase 2 启用）。图片 v1a 占位 1×1 白贴图；**Text Phase 2 ✅**（font atlas）。

### 2.10 文字渲染链（v1a Phase 2，§9/§14）
- **Rust 笔位权威 + Unity 纯光栅**（偏离 fgui 的 advance/行高，§9.1 跨平台根）。blob text_arena 每 text 节点 = `font_size:u32|color:f32×4|glyph_count:u32|glyphs[{codepoint:u32,pen_x:f32,pen_y:f32}]`；pen=GO-local（content 偏移 Rust 烤进），pen_y=`line.y+line.baseline`，**不 re-base**（pen 已节点局部，与 mesh re-base 不同）。
- Unity `TextRasterizer.BuildMesh`：`RequestCharactersInTexture(串,font_size)` 填 atlas → 每 glyph `GetCharacterInfo((char)codepoint,font_size)` 取 UV 四角 + 像素 box(`minX/maxY/maxX/minY`)，quad 摆 `pen+box`（y-down：`top=pen_y−maxY,bottom=pen_y−minY`），顶点序 BL/TL/TR/BR 对齐 fgui `DrawGlyph`。**不用** `CharacterInfo.advance` / `fontSize×1.25` 行高。
- **必修坑**：`Font.textureRebuilt` 静态事件 → `s_fontVersion++` → MirrorPool 下帧检测版本变 → 强制 text 节点重 BuildMesh（atlas rebuild 后 glyph UV 变，不监听画花字）。照搬 fgui `DynamicFont.cs:356-375`。
- material key=(program=1, `font.material.mainTexture`, mask_context)；texture=动态字体 atlas。

### 2.11 rect mask `_ClipBox`（v1a Phase 2，§8.6/§14）
- Rust：batch DFS 算**嵌套 clip 交集**（祖先 clip 链累乘交，disjoint→零面积 Rect 非 None），emit clip 表 `context_id→绝对 design rect`；修 v0「只裁最内层」bug。mask_context 进 material key。
- Unity：design rect→world（**根 transform** `TransformPoint` 两角，非逐 clipper 矩阵——clip 是绝对 design 非 clipper-local）→ `_ClipBox=(-cx/hw,-cy/hh,1/hw,1/hh)`（照搬 fgui `UpdateContext.cs:105-156`）；零 half→safe-blank `(-2,-2,0,0)` 防 div0。MirrorPool 每 ctx 每帧首次 SetClipBox（fgui `firstMaterialInFrame`）。shader CLIPPED：`clipPos=TransformObjectToWorld(pos).xy×zw+xy`，`col.a*=step(max(abs(clipPos)),1)`。
- **坐标模型**：blob local_x/local_y + clip_rect 均**绝对 design**（layout 累加父 origin）；后端 flatten 挂根 GO，根 transform 一次性映射 design→world。nesting+父相对坐标留 v1c（transform 继承/事件）。

### 2.12 打包器 + 包格式（v1b.1，§12/§5.5）
- **`.pkg.bin` 是 Rust-internal**：`loomgui_pkg` 写、core runtime 读，**C# 永不解析**（Unity 只读文件→bytes→`load_package`）。与 frame blob（Rust↔C# 跨语言契约）本质不同——无需 C# reader/跨语言字节对齐，style 可直接 bincode 投影。
- v1 格式（扁平 + stringTable，LE）：Header 28B（magic `LPKG`=0x474B504C + version=1 + flags + nodeCount + stringCount + rootSizeX/Y）+ StringTable（u16 len+UTF8，**只** text content + image src）+ NodeBlock（每节点 parentIndex i32 + kind u8 + styleLen u32 + `bincode(ResolvedStyle)` blob + textIdx/srcIdx u16；NULL_IDX=0xFFFF；kind 0=Container/1=Button/2=Image/3=Text）。indexTable/压缩/分支推 formatVersion=3+（v1x-deferred §6；v1b.3 已占 v2=AtlasSection）。
- **StyleRecord = bincode(ResolvedStyle)**：taffy 开 `serde` feature（§3.7），ResolvedStyle/TextAlign 加 `Serialize/Deserialize/PartialEq` 派生——穷尽由派生保证（加字段编译期强制覆盖 encode/decode，R3≈0）。font_family 随 blob 走（不进 stringTable）。
- `asset::write_package(&Scene,root_size)->Vec<u8>` / `read_package(&[u8])->Result<(Scene,root_size),PkgError>`（常驻，不依赖 parse）。read 全 `Result` 无 panic 跨 FFI（Reader 截断保护）。版本协商：magic + formatVersion∈[1,1]（fgui 缺，主设计 §12.2 要）。
- **`Scene::build(&[(Option<usize>,NodeKind,ResolvedStyle)])`** 共享建树（常驻，不依赖 parse）——`build_scene`（parse 路径，gate）与 `read_package`（runtime 路径）共用，防建树逻辑分叉（R2）。NodeId=entries 下标；children 按 DFS 先序填。
- **parse feature gate**：`scraper`/`cssparser` optional + `parse` feature（default on）。gate 在 parse 后：`parse/` 模块 + `style::cascade` + `build_scene`/`gather_rec` + `Stage::load_inline` + 用 parse 的测。常驻：`ResolvedStyle`/`TextAlign`/`Scene::build`/`mapping`/layout/render/text/scene/stage(除 load_inline)/asset。`loomgui_ffi_c`：`load_package` 常驻、`load_html` gate；**dev .dll 仍带 parse**（PlayMode inline 迭代要），gate 价值=架构正确 + 将来精简 build。构建矩阵门：`cargo build -p loomgui_{core,ffi_c} --no-default-features` 皆编。
- **黄金等价测**（最强门）：`pkg→load_package→render_json` == `inline load_inline→render_json`（包路径与 inline 渲染逐节点等价，验收 #6）。fixture 覆盖 div/text/img/rect mask。
- `loomgui_pkg` CLI（不引 clap，`std::env::args`）：`pack(html,css,root_size)` = `parse_html→parse_css→resolve_styles→build_scene→write_package`。packager **不**加载字体/不 solve/不 render。
- **v1b.3 图集打包**：formatVersion 1→**2**（MIN=MAX=2，旧 v1 拒）；NodeBlock 后追加 **AtlasSection**（atlas_count + 每 atlas{filename_idx,w,h} + sprite_count + 每 sprite{src_idx,x,y,w,h region}）。`write_package(&Scene,root_size,&AtlasSection)` / `read_package -> (Scene,root_size,AtlasSection)`。`pack(html,css,root_size,res_dir) -> PackedPackage{pkg_bytes, atlas_png, atlas_filename}`：`image` crate 解码散图（§3.8）→ shelf 打包（NPOT/无旋转/trim/单图集，atlas_w=max(512,最宽)）→ blit 进 atlas buffer + 编码 atlas.png。缺图 build-time `Err`。详见 §2.14。

### 2.13 纹理注册层（v1b.2，§8.3/§14.3）
- **TexId 是 FFI 代价**：fgui 单进程按 NTexture 对象引用纹理；LoomGUI Rust core + Unity 跨 FFI，blob 必用整数 id → core 持 `TextureRegistry{src→TexMeta{tex_id,w,h}, next_id}`（per Stage，纯 id+维度表无 GPU），Unity 持 `Dictionary<tex_id,Texture2D>`，握手靠 register/collect。
- **registry 由 measure 强制**（非为 blob）：真实尺寸测量要每 src 的 w/h → core 必有 src→dims 表；register(src,w,h) 主因报维度，tex_id 顺带分配。故即便 blob 改带 src 串，core 仍要 register。
- **注册握手**（LoomStage Awake，`_usePackage` 后、首 tick 前）：collect srcs → Unity `File.ReadAllBytes`+`Texture2D.LoadImage` → `register_texture` → 建 `_texMap`；缺图/坏图 try/catch→LogError+跳过（白占位）；OnDestroy Dispose 全部（`isPlaying?Destroy:DestroyImmediate`，坑 11 ExecuteAlways 泄漏）。
- **5-hop tex_id 流**：render 查表填 `NodePayload::Mesh.texture` → blob v3 `col_tex_id` 列 → C# `FrameBlob.TexId(i)` → MirrorPool `texMap[tid]` 查表 → `mm.Get(0,tex,ctx)`。tex_id=0 哨兵→白占位。shader/MaterialManager **零改**（`tex2D×v.color` 已支持纯色块 tex_id=0 + 真图 tex_id>=1 两路）。
- **v1b.3 演进为图集模型**（见 §2.14）：散图外部文件 + runtime register 被 packer 期 atlas 打包取代；`TextureRegistry` 改 `src→TexMeta{tex_id,**uv_min,uv_max**,w,h}`（core load 时 `build_registry` 建，**无 runtime register**）。§2.13 的 register/collect 握手已删（FFI 改 atlas_count/info）。

### 2.14 图集层（v1b.3，§8.3/§12.3）
- **打包器期打图集**（fgui 模型）：`loomgui_pkg` 读散图（`image` crate，§3.8）→ shelf 打包 → blit+编码 atlas.png + AtlasSprite 表写进 .pkg.bin v2。**散图不再进 StreamingAssets**（已烤进 atlas）；StreamingAssets 放 `.pkg.bin` + `atlas.png`。
- **core 持 UV region**（去引擎化）：`build_registry(&AtlasSection)` 从表建 `src→TexMeta{tex_id,uv_min,uv_max,w,h}`（atlas[0]→tex_id 1，所有 sprite 共享；`uv_min=[x/aw,y/ah]`、`uv_max=[(x+w)/aw,(y+h)/ah]`，y-down convention）。render 按 uv 烤 quad 4 角（§2.6）。
- **同图集共享 1 Texture2D**：Unity `_texMap[atlas_tex_id]` = 1 张 atlas；多 sprite 同 tex_id → MaterialManager 返同 Material（key 含 texture）→ **SRP Batcher 批合**（CPU 效率）+ 可选 URP Dynamic Batching 降 draw call。
- **batching 认知（v1b.4 更新）**：v1b.3 时 mesh 不合并（每节点独立 MeshRenderer，仅同 Material/SRP Batcher，不保证 N→1 draw call）。**v1b.4 起 core 显式合并**（§2.6 reorder+merge）→ 连续同 atlas sprite 段真 N→1 draw call。**认知修正（坑 22）**：fgui DoFairyBatching 本身**不合并 mesh**（只重排 sortingOrder，靠 Unity Dynamic Batching 隐式合——不可控+URP 下与 SRP Batcher 互斥）；LoomGUI core 显式合并是补 fgui 没做的。
- **blob v3 不变**：atlas 子区 UV 是 mesh_arena per-vertex uv 的不同值（非格式改）。UV 方向沿用 v1b.2 convention（packer region y-down px → core uv → root 一次性 y-flip），PlayMode 验方向正。
- **defer（v1b.5+）**：rotation（UV 修正 §8.2 公式 `new_y=yMin+uv.x-xMin; new_x=xMin+yMax-uv.y`）、trim（originalSize/offset）、多图集（sprite 带 atlas_idx）、refcount/on_release（§12.4，atlas 随 Stage）。~~mesh 合并~~（v1b.4 ✅）。

### 2.15 事件/命中/输入层（v1c.1，§10）
- **input.rs**（常驻）：`PointerEvent`/`EventRecord`（#[repr(C)] FFI POD）+ `PointerState` 单指针状态机（`process` 产 Down/Up/Move/Click/RollOver/RollOut；click ~10px 阈值 §10.3）+ **hover/active 祖先链**：`set_hovered_chain`/`set_active_chain` 沿 parent 链设 target+所有祖先（对齐 fgui rollOverChain + CSS :hover「后代上→祖也 hover」，坑 29）。
- **hit.rs**：`hit_test(scene, point) -> Option<NodeId>` 逆等效绘制序（读 `node.style.order` 降序 + `Reverse` 同 order 后绘制者顶层），`layout_rect` AABB + clip 子树门控 + pointer-events:none 跳自身测子 + disabled 仍命中（active/click 在状态机层抑制，§4.4 偏离 fgui 支持 :disabled hover 反馈）。
- **style/dynamic.rs**（常驻；selector 类型从 parse 迁此修 parse-gate，坑 28 同源）：`DynamicRuleTable`（含伪类规则，打包器 `extract_dynamic_rules` 抽）+ `match_element_with_state`（后代链 + 每 compound 伪类状态门）+ `rematch_pseudo_classes`（全量节点重 cascade 仅动态规则子集，base_style 重起，§5.3 不缓存；invalidation set 留 v1e 撞墙）。
- **Stage tick 管线**（§15）：solve→process(hit+状态diff)→cur_hit→rematch→render。**solve 必须在 hit 前**（§15「命中用本帧刚 solve 布局」；brief §4.5 写 process→rematch→solve 是笔误，T8 修前移 solve）。事件回调改布局延下帧（防反馈环）；rematch 改 layout 也延下帧（sample 伪类全视觉字段不受影响）。
- **FFI**（pull 模式绕 §14.2 IL2CPP 回调坑）：`set_input`/`borrow_events`/`is_pointer_on_ui`/`set_node_disabled` 全常驻不 gate。listener 在 C# 侧（对齐 fgui），核心只算命中+产事件+伪类重匹配。
- **Scene/Node 加**：`base_style`(不变)/`classes`/`id_attr`(CSS id，非 `Node.id: NodeId` 占用)/`touchable`/`hovered`/`active`/`disabled` + `dynamic_rules`。pkg.bin v2→v3 加 DynamicRuleSection（bincode DynamicRuleTable）+ NodeBlock classes/id_attr 段。

### 2.16 事件路由层（v1c.2，§10.2 方向 A）
- **方向 A（架构决策）**：bubble/capture 路由 + listener 表在 **C# 业务侧**（`LoomEventHandler`），非核心。核心只保留命中 hit_test + 命中 diff（hover/active 状态 + RollOver/Out 产出）+ 伪类 rematch。主设计 §10.2/§6.3 line251/§15 已修订（删 `Node.listeners`，路由降级业务侧）。判据：fgui 整个事件管线（命中+状态机+rollOver diff+bubble+click）全在 C# `Stage.cs` 业务侧（非核心）；`stop_propagation` 是回调副作用必须在 C#；核心最小改动。
- **核心↔C# 边界**：核心产 target 事件（`EventRecord{node_id=target}`）+ RollOver/Out 多目标 diff；C# 沿 `node_parent` 链 bubble/capture。**EventRecord 零改**（node_id=target，C# 按 event_type 分流：Down/Up/Move/Click→BubbleRoute，RollOver/Out→DirectDispatch）。只新增 `node_parent` FFI。
- **hover_diff 祖先链 diff（点1，修坑 29 嵌套多发）**：v1c.1 单点 diff（旧 target RollOut+新 target RollOver）→ 两链 diff（`last_hovered_chain: Vec<NodeId>` + `ancestor_chain()` 沿 parent 至 root；旧链独有 RollOut、新链独有 RollOver、共同祖先段不产——鼠标从父进子父不 RollOut）。照 fgui `HandleRollOver`（Stage.cs:1315）。hovered 状态仍 `set_hovered_chain`（rematch 用）。
- **C# 路由（照 fgui `EventDispatcher`）**：`BubbleRoute`（capture 根→target 反向**全跑不检查 stop**，照 BubbleEvent line302-311 + bubble target→root 正向 stop break line315-328）+ `DirectDispatch`（RollOver/Out 单节点 capture+bubble 不沿链，照 `InternalDispatchEvent`）+ `AncestorChain`（node_parent 缓存，sentinel 0xFFFFFFFF 止）+ `EventContext`（对象池 Stack + target/currentTarget/phase + StopPropagation/PreventDefault，Get 只重置 stop/prevent **不重置 payload**，照 fgui）+ `EventBridge`（多播 _bubble/_capture，Add 内 `-=cb;+=cb` 去重）+ 委托引用 remove（非 ListenerId）+ `SetHandle`（赋 _handle + 清 _parentCache，**每次 load 调**非只 Awake）。
- **FFI 加**：`loomgui_node_parent(h: *const StageHandle, node_id: u32) -> u32`（根/越界/无 scene → 0xFFFFFFFF；常驻不 gate 坑21）。version v1c.2。
- **两机约束（v1c.2 执行）**：core cargo test 本机 TDD 闭环；C# 代码本机写不编译（无 Unity），家里机 EditMode+PlayMode 验。改 Rust 后重编+commit .dll（坑10 两机变体）。subagent-driven 6 task 全 Approved。

## 3. 依赖 API 适配踩坑（v0 最大教训）

> **plan/brief 写的 API 草稿常与实际 crate 版本不符**。遇编译错按本节对照，**勿硬改依赖版本**，按 crate 实际源码（`~/.cargo/registry/src/<crate>-<ver>/src/`）调。

### 3.1 taffy 0.5（layout/mod.rs）
- **无 `MeasureFunc::Boxed`**。用 `TaffyTree<NodeContext>` + `new_leaf_with_context(style, ctx)` + `compute_layout_with_measure(root, Size::MAX_CONTENT, FnMut)`。
- measure 闭包签名：`FnMut(Size<Option<f32>>, Size<AvailableSpace>, NodeId, Option<&mut NodeContext>, &Style) -> Size<f32>`。`known.width` 是 `Option<f32>`（Some=约束宽，None=不限）。
- **闭包可借 `&font`**（FnMat 调用期存活，非 `'static`）→ **不需要 `Arc<Font>`**（v0 一度误判要 Arc，实际单 FnMut 借用合法）。
- `Size::MAX` → `Size::MAX_CONTENT`。
- 根 size setter 用 `Dimension::Length`（`Style.size` 是 `Size<Dimension>`）。
- `Style` **无 `order` 字段**（CSS order 无法存 taffy；留 `ResolvedStyle.order` 待 v1 消费）。

### 3.2 ttf-parser 0.20（text/layout.rs）
- **`glyph_hor_advance(GlyphId) -> Option<u16>`**（非 `glyph_advance_width`，返回 u16 非 i16）。
- **kerning 在 `kern::Subtable`**：`face.tables().kern.subtables` 遍历（取 horizontal + 非状态机子表），`.glyphs_kerning(GlyphId, GlyphId) -> Option<i16>`。`Subtables` 是 `Copy`。
- `glyph_index(ch) -> Option<GlyphId>`（`GlyphId(pub u16)`）。
- `glyph_bounding_box(GlyphId) -> Option<Rect{i16}>`。bearing 用 `x_min`/`y_max`。
- `ascender()/descender()/line_gap()/units_per_em()` 在 `Face` 上。
- `Face::parse(&'static [u8], 0)`——v0 用 `Box::leak` 拿 `'static`（单字体 OK，多字体 v1 换 owned wrapper）。**`.ttc`（TrueType Collection）第二参=collection index**：文泉驿微米黑 .ttc index 0 = Micro Hei Regular（collection 含 2 face）。`.ttf` 单文件 index 0。

### 3.3 cssparser 0.34（parse/css.rs）
- **不能用 NestingParser + parse_one_rule 草稿**。
- `DeclParser` 需实现三 trait：`DeclarationParser + QualifiedRuleParser + AtRuleParser`（`RuleBodyItemParser` 要求三者）。
- 用 `StyleSheetParser` 迭代器替代 `parse_one_rule` 循环。
- `parse_block` 参数是 `ParserState` 非 `SourcePosition`。
- v0 不解析 @ 规则（`AtRuleParser` 默认拒）。

### 3.4 scraper 0.19（parse/dom.rs）
- `Html::parse_document` → `select("body")` → `children()` 迭代。
- `ElementRef::value()` 取 Element，`.attrs()` 取属性迭代。
- `<img>` 是 void 元素（无闭合标签），src 从 `attrs` 取非 text。

### 3.5 csbindgen 1.9（loomgui_ffi_c/build.rs + 生成 LoomGUIBindings.cs）
- 默认生成 **`internal`** 类型（`Native` 类、`StageHandle` 结构）→ 跨程序集（LoomGUI.Bindings→LoomGUI.Runtime）访问须 `[assembly: InternalsVisibleTo("LoomGUI.Runtime")]`（放 AssemblyInfo.cs）。
- 类型映射：`*const u8`→`byte*`、`*mut usize`→`nuint*`、opaque `*mut T`→`T*`（**类型化指针非 IntPtr**）。`csharp_use_function_pointer(false)` 切 Mono 模式。
- `CString::as_ptr()` 返 `*const c_char`(i8)，签名为 `*const u8` 时须 `as *const u8` cast。
- build.rs 跑两次（OUT_DIR 必成 `.expect`；Unity 目录那次失败要 `cargo:warning=` 勿 `let _ =` 吞错）。
- C# `fixed(T* p=&localVar)` **非法**（CS0213 "already fixed"）——局部栈上已固定，直接 `&localVar` 传；`fixed` 只 pin 托管对象（数组/string）。

### 3.6 Unity 6.5（6000.5）C# API
- `Object.GetInstanceID()` **废弃**→`GetEntityId()`，后者返 **`EntityId`（非 int）**，`EntityId→int` 隐式转换**也废弃**（"将来不能 int 表示"）。**绕开整条**：缓存 key 直接持 Object 引用（`Dictionary<Texture,...>`，Unity 对象引用同一性），别碰任何 id API。
- **EditMode 禁 `Object.Destroy`**（报 "Destroy may not be called from edit mode"），须 `DestroyImmediate`。`[ExecuteAlways]` 组件生产代码 EditMode 也跑 → `Application.isPlaying ? Destroy : DestroyImmediate`（Mesh 是独立 UnityEngine.Object，GO 销毁不连带，须显式销毁防泄漏）。
- `Camera.nearClipPlane` **须 >0**（负值抛异常）。
- Unity 开着**锁 native `.dll`**——重建/拷 `.dll` 须先关 Unity（锁文件 `Temp/UnityLockfile` 可能残留非真锁，以能否 `rm` 为准）。
- 生成物 gitignore：`.slnx`(6.5 解决方案)、csbindgen `.cs` 绑定及其 `.cs.meta`；`.dll`（`!**/Plugins/**/*.dll` 白名单）入库；`.meta` 是 Unity 资产元数据须入库（**implementer 提 .cs 易漏 .meta**——Unity 关着时不生成，坑 13）。
- **动态字体 API**（Text 光栅）：`Font.RequestCharactersInTexture(string, fontSize, FontStyle)` 填 atlas（必先调，否则 GetCharacterInfo 恒 false）→ `Font.GetCharacterInfo(char, out CharacterInfo, fontSize, FontStyle) -> bool` 取 `minX/maxY/maxX/minY`（像素 box）+ `uvBottomLeft/uvTopLeft/uvTopRight/uvBottomRight`。`CharacterInfo.advance` 存在但 LoomGUI **不用**（Rust 笔位）。`Font.textureRebuilt` 是**静态**事件（register/OnDestroy 解绑防泄漏）。
- **`HideFlags.DontSaveInEditor`**：`[ExecuteAlways]` 程序生成 GO 标之防被存进场景（否则 EditMode dirty 场景 + Play/Stop 累积残留，坑 11）。
- **`Mesh.SetVertices(List)`/`SetUVs`/`SetColors`/`SetTriangles(List)` overload**：零 per-frame 数组 alloc（vs `SetVertices(Vector3[])`）。`List.Clear()` 保 Capacity，warm-up 后复用零 alloc。
- **shader keyword**：`#pragma multi_compile _ CLIPPED`（两 variant 都编，`EnableKeyword` 切换生效）**非** `shader_feature`（未启用的 variant 会被 strip → clip 静默失效）。

### 3.7 taffy 0.5.2 serde + bincode 1.x（style/resolved.rs + asset/mod.rs，v1b.1）
- taffy 0.5.2 有 **`serde` feature**：`Style`（style/mod.rs:189）及全部字段类型（geometry/dimension/flex/grid/alignment）都 `#[cfg_attr(feature="serde", derive(Serialize,Deserialize))]` + `#[serde(default)]`；`Style` 还派生 `PartialEq`。开 `taffy = { version="0.5", features=["serde"] }` 后，含 `taffy_style: taffy::style::Style` 的 `ResolvedStyle` 能整体 `#[derive(Serialize,Deserialize,PartialEq)]`。
- bincode 1.x：`bincode::serialize(&x)->Vec<u8>` / `bincode::deserialize::<T>(&bytes)`。`#[serde(default)]` 在 bincode（位置编码无缺字段概念）下透明。用于包格式的 StyleRecord——穷尽由 serde 派生保证，比手写枚举 taffy 30+ 字段稳健（R3≈0）。
- bincode 格式随 taffy/bincode 版本——升级时 bump 包 `formatVersion`。

### 3.8 image 0.25（loomgui_pkg，v1b.3）
- **`save_buffer_to_memory` 不存在**（plan 草稿写错）→ 用 `RgbaImage::write_to(&mut std::io::Cursor<Vec<u8>>, ImageFormat::Png)` 编码 PNG 到内存。
- 解码：`image::open(path)?.to_rgba8() -> RgbaImage`（像素+w/h）；合成 atlas：`RgbaImage::from_raw(w, h, buf)` 建图；回查测：`image::load_from_memory(&bytes).to_rgba8()`。
- Cargo：`image = { version = "0.25", default-features = false, features = ["png"] }`（仅 png 最小依赖；**只在 packer，core 不碰像素**）。
- 教训：plan 草稿的 crate API 名常错（本例 `save_buffer_to_memory`）→ 实现 RED 阶段验实际 API（`~/.cargo/registry/src/<image>-<ver>/src/`）。

### 3.9 unicode-linebreak 0.1（text/layout.rs，v1b.5）
- **`linebreaks(s: &str) -> impl Iterator<Item=(usize, BreakOpportunity)>`**（非草稿 `Vec<(usize, BreakType)>`——返**迭代器**非 Vec，需 `.collect::<Vec<_>>()`）。
- **`enum BreakOpportunity { Mandatory, Allowed }`**（非草稿 `BreakType`——变体名同但**枚举名不同**）。
- 返回 `usize` = **byte offset**（非 char index），升序；offset 语义 = 可在该 byte offset 处断（前段 `content[..offset]`，后段 `content[offset..]`）。unicode-linebreak 在空白**后**断 → segment 自含尾空白 → 行首无多余空格。
- 用法（layout.rs:194+）：`linebreaks(content).collect()` → 按 offset 切 segments `Vec<(&str, BreakOpportunity)>` → greedy fill（累加 seg 宽超 max_w 换行，Mandatory 强制结束行）。
- 教训：brief/草稿写 `Vec<(usize, BreakType)>` 实际是 `impl Iterator`+`BreakOpportunity`（坑 1/2/8 同源）→ 实现 RED 阶段验 `~/.cargo/registry/src/<unicode-linebreak>-<ver>/src/`。

### 3.10 Unity Input System 1.19（LoomInputCollector.cs，v1c.1）
- **新 API**：`Mouse.current.position.ReadValue()`（左下原点 screen 像素，同旧 `Input.mousePosition` 语义）/ `Mouse.current.leftButton.wasPressedThisFrame`·`wasReleasedThisFrame`（vs 旧 `Input.GetMouseButtonDown/Up`）。
- **双路径**：`#if ENABLE_INPUT_SYSTEM`（Player Settings Active Input Handling=New/Both 定义此宏）走新 API，else 旧 `UnityEngine.Input`。asmdef references 加 `"Unity.InputSystem"`（非 `UnityEngine.InputSystemModule`——那个名错编译失败）。
- 教训：plan 选旧 Input 但工程切了 Input System package → 运行时 `InvalidOperationException`（坑 28）。Unity Input System 是 package（assembly `Unity.InputSystem`），非UnityEngine 内置。

## 4. AI 可预测性核心约束（首要准则，勿违背）

> LoomGUI 根本目的 = AI 驱动界面拼装。HTML 作 DSL 让 AI 能编辑+预测渲染。以下约束是 AI 可预测性的根基，违背即损害核心目的。

1. **div 默认 `flex-direction: column`**（§4.1）。`ResolvedStyle::default()` 设 `FlexDirection::Column`（taffy 默认是 Row！）。CSS 显式 flex-direction 无条件覆盖。AI 对 div「垂直堆叠」的先验成立。
2. **div 永远是 flex 容器，只装 flex item**。无浏览器 block/inline flow。文本+图混排进 `<l-rich>`（v1.x）。
3. **div/button 裸文本 → Text 子节点**（§4.2）。`<div>标题</div>` 产出 Container + Text 子「标题」（文本是 flex item），**不丢弃**。
4. **行内混排报错**（文本+元素同在）。解析期 Err，提示用 span/l-rich。
5. **围栏外元素报错不降级**。parse 白名单 `[div/span/img/button/l-container]`，其它 Err。
6. **margin 不折叠**（flex 语义）。子项间距用 `gap`，别用 margin（margin 在 LoomGUI 求和、Chrome block flow 折叠，Chrome 预览会骗 AI）。
7. **glyph 绝对坐标**（§9.2）。后端拼 quad 零累加。
8. **坐标系左上原点、y 向下**（§8.1）。核心代码无 `height-y` 翻转（翻转在后端根 Stage 一次性）。

## 5. v0 踩坑记录

### 坑 1：taffy 0.5 `MeasureFunc::Boxed` 不存在
**症状**：layout brief 写 `MeasureFunc::Boxed` 闭包，编译报无此变体。
**根因**：taffy 0.5.2 改用 `TaffyTree<NodeContext>` + `compute_layout_with_measure` FnMut 分发。
**解决**：见 §3.1。`Arc<Font>` carry 作废（FnMut 借用合法）。
**教训**：brief 的 API 草稿是起点非权威，按编译器 + crate 实际版本调。

### 坑 2：ttf-parser 0.20 advance/kerning API 改名
**症状**：`glyph_advance_width`/`kerning_for` 编译失败。
**根因**：0.20 改名 `glyph_hor_advance`（返 u16）；kerning 移到 `kern::Subtable`。
**解决**：见 §3.2。
**教训**：ttf-parser 跨版本 API 变动大，查 `~/.cargo/registry` 源码确认。

### 坑 3：默认 flex-direction 没落地 column
**症状**：未显式写 flex-direction 的 div 水平排列，违反 §4.1。
**根因**：`ResolvedStyle::default()` 用 taffy `Style::DEFAULT`（flex_direction=Row）。实现者一度用测试 CSS 掩盖（加 flex-direction:column 让测试过），final review 抓出。
**解决**：Default impl 设 Column；CSS 显式声明无条件覆盖（时序 default→apply_decl）。加 `default_div_is_column` 回归测试。
**教训**：AI 可预测性核心约束必须在**默认值层**落地，不能靠测试 fixture 掩盖。

### 坑 4：围栏外元素静默降级 Text
**症状**：`<video>` 等围栏外 tag 被当 Text 节点。
**根因**：scene 层 `_ => NodeKind::Text` fallback。
**解决**：parse 层白名单报错（执法点在 parse 非 scene），scene 删 fallback 改 `unreachable!`。
**教训**：「报错不降级」类约束执法点要对（parse），下游信任输入。

### 坑 5：div/button 裸文本被丢弃
**症状**：`<div>标题</div>` 无文本输出。
**根因**：scene `build_rec` 对 Container 不处理 `el.text`。
**解决**：`build_text_child` 生成 Text 子节点（继承父 8 文本字段，size=Auto 不污染高度）。
**教训**：spec §4.2「Text 叶子是 Container 子节点」——裸文本该成 flex item 子节点，非丢弃。

### 坑 6：cascade specificity 排序方向反
**症状**：多规则命中时低 specificity 胜。
**根因**：`match_element` 返回降序，直接顺序 apply 让低优先级后写覆盖高优先级（反了）。
**解决**：`resolve_styles` 加 `sort_by_key` 升序（高 specificity 后 apply 胜，稳定排序保同级 source 顺序）。
**教训**：接 `match_element` 时核对排序方向；CSS cascade 是高 specificity 胜。

### 坑 7：后代选择器只查直接父
**症状**：`div.a span` 在 `<div class=a><div><span>` 不命中。
**根因**：`matches()` 只查 parent 不递归祖先；`Combinator` 字段未用。
**解决**：`match_compound_chain` 递归祖先（Descendant 沿父链搜+回溯，Child 只直接父）。附带修 `parse_selector` 空格降级 Child bug。
**教训**：围栏声明「后代/子代」选择器就要真实现递归祖先。

### 坑 8：snapshot 绑系统 arial.ttf
**症状**：Linux CI（DejaVuSans 无 arial）snapshot 漂移。
**根因**：测试字体试系统路径。
**解决**：锁仓库内 `tests/fixtures/DejaVuSans.ttf` + `env!("CARGO_MANIFEST_DIR")`；fixture 用 ASCII（DejaVuSans 无 CJK）。
**教训**：测试产物跨平台一致就锁仓库内资源；CJK 渲染验证留 v1（需 CJK 字体）。

### 坑 9：color_tint 把背景色块涂黑（v1a T8）
**症状**：PlayMode 红背景块渲成黑色。
**根因**：v0 `ResolvedStyle::default().color=[0,0,0,1]`（CSS `color` 默认黑，是**前景/文本色**）；blob 烘焙 `bg×color_tint×alpha` 把红背景乘成不透明黑。
**解决**：build_blob **不乘 color_tint**——顶点色 = background_color，仅 `alpha×node opacity`。color_tint 是文本色（Phase 2 文本用）。
**教训**：mesh colors 已是最终色（bg-color / 图片白），别再叠 color_tint；§4.2b「tint×alpha 烘焙」指**文本/图片** tint，非背景色块。

### 坑 10：Rust 改 blob 格式后 .dll 没换 → C# 静默拒帧不渲染（v1a Phase 2）
**症状**：PlayMode **啥都不渲**（红块文字全无）、Console **干净无错**。
**根因**：Plugins 里 `.dll` 是旧的（Phase 1），产 **v1 blob（version=1）**；T1 起 C# `FrameBlob.IsValid` 只认 version==2 → `MirrorPool.Sync` 第 1 行 `if(!IsValid)return` **静默早退**。Unity 开着锁 .dll，编译后没换。
**解决**：`cargo build --release` → **关 Unity**（锁 .dll）→ `cp target/release/loomgui_ffi_c.dll Plugins/LoomGUI/` → 重开。
**教训**：**任何 Rust FFI 改动（尤其 blob/ABI 格式）后，PlayMode 验前必重编+换 .dll**。症状"全不渲+Console 干净"先怀疑 stale .dll（`md5sum` 对比 fresh build）。Unity 开着锁 .dll，换 .dll 必关 Unity。

### 坑 11：ExecuteAlways 程序生成 GO 累积泄漏（v1a Phase 2）
**症状**：Play/Stop 反复 + domain reload 后 `loom_node` GO 在 Hierarchy 累积、内存泄漏。
**根因**：`[ExecuteAlways]` EditMode 也跑，MirrorPool 产的 `loom_node` GO 挂 root 下被**存进场景**；每次 Awake 新 `_pool` 丢旧引用 → 孤儿 GO 清不掉。
**解决**：GO/Mesh 标 `HideFlags.DontSaveInEditor`（不入存盘）+ Awake 开头清 root 下 `loom_node` 孤儿 GO。
**教训**：ExecuteAlways + 程序生成 GO 必加 DontSave + 开局清孤儿；OnDestroy 的 `pool.Clear` 只清当前 run 的，跨 run 孤儿要 Awake 清。

### 坑 12：手搓 blob 测 fixture 写 AoS，多节点读串列（v1a Phase 2）
**症状**：EditMode 跑 `MirrorPoolFlattenTests` 报 `SetTriangles: idx 非三的倍数`。
**根因**：C# 手搓 2 节点 blob 写 **AoS**（node0 全字段、node1 全字段）但列 offset 按 1 节点 elemSize 递进、`FrameBlob` 读 **SOA**（列优先 `ColOff(idx)+i*elemSize`）→ node1 每字段读串一位，mesh_off 落到 node0 mesh_len → idx 读成垃圾。
**解决**：fixture 列 offset 按 `NodeCount×elemSize` 递进、数据列优先写（镜像 `blob.rs`）。单节点 fixture AoS≡SOA 不受影响（故 Phase 1 没暴露）。
**教训**：手搓 blob byte[] 测必 SOA 列优先，与 `blob.rs`/`FrameBlob` 一致；多节点才暴露（单节点掩盖）。

### 坑 13：implementer 提 .cs 漏 .meta（v1a Phase 2）
**症状**：合 main 后 4 个新 `.cs` 的 `.meta` 没入库（ClipMath/ClipBoxTests/FrameBlobV2Tests/MirrorPoolFlattenTests）。
**根因**：Unity 关着时 import 不生成 `.meta`；subagent 提 .cs 时 Unity 未开 → .meta 后生成、未 add。TextRasterizer.cs.meta 这次提了（那次 Unity 开着），其余漏。
**解决**：合 main 后补提交漏的 .meta；或 implementer 提前确保 Unity 开过一次生成 .meta。
**教训**：Unity `.meta` 随 `.cs` 入库；subagent 流程里提 .cs 后 controller 验 `.meta` 是否齐（Unity 生成的资产元数据，缺则 GUID 不稳）。

### 坑 14：Unity GetCharacterInfo 要 codepoint 非 glyph_id（v1a Phase 2）
**症状**：text_arena 只有 ttf `glyph_id`，Unity 取不到字。
**根因**：Unity `Font.GetCharacterInfo(char, ...)` 按 **Unicode 码点**，非 ttf glyph_id；ttf glyph_id 是字体内部字形索引。
**解决**：Rust `Glyph` 加 `codepoint:u32`（`measure_text` 遍历 char 时填），text_arena 送 codepoint，Unity `(char)codepoint` 调 GetCharacterInfo。
**教训**：引擎字体 API 多按码点；核心 Glyph 须同时持 glyph_id（ttf 直连后端）+ codepoint（引擎字体 API）。

### 坑 15：新二进制格式 magic 撞既有格式（v1b.1）
**症状**：v1b.1 包格式初拟 magic `"LOOM"`(0x4D4F4F4C)，与 frame blob 的 `MAGIC`（blob.rs）**完全相同**。
**根因**：两种格式独立，但 magic 是唯一识别码；撞了 magic→校验形同虚设（误传 frame blob 给 `load_package` 会过 magic 检查再挂错）。
**解决**：包改独立 magic `"LPKG"`(0x474B504C，磁盘字节 `4C 50 4B 47`)。planning 期 grep 现有 magic 才发现。
**教训**：新增二进制格式先 `grep -r 'MAGIC\|0x4D4F4F4C' src/` 确认 magic 唯一；formatVersion 是「同格式的版本」，magic 是「这是哪种格式」，两者正交。

### 坑 16：FFI 返 String::as_ptr() 无尾 NUL，C# NUL-scan 读越界（v1b.2）
**症状**：`image_src_at` 返 src 串指针，C# 用 `PtrStringAnsi(ptr)` 或 Rust 测用 `CStr::from_ptr` 读 → 越界读到 `Utf8Error{valid_up_to>N}` 或乱码。
**根因**：Rust `String`/`Vec<u8>` 缓冲**无尾 `\0`**；`PtrStringAnsi`/`CStr` 靠 NUL-scan 会越过 stage 缓存末尾。
**解决**：FFI 同时返 `*out_len`（字节长）；C# 用 `Encoding.UTF8.GetString(ptr,(int)len)`，Rust 测用 `slice::from_raw_parts(p,len)`+`from_utf8`。同 `borrow_frame` 的 ptr+len 契约。
**教训**：Rust FFI 返字符串一律 ptr+len（不靠 NUL）；C# 侧禁用 NUL-scan 读法。任何新 len-based 串返回（font/资源名）照此。

### 坑 17：bump blob version 须同步所有手搓 C# fixture，漏一个 4 字节 skew（v1b.2）
**症状**：blob v2→v3 加 tex_id 列，Rust `blob.rs` + C# `FrameBlob.cs` 都改了，但 `FrameBlobV2Tests.BuildMinimalV2Blob` 只升 header（14 列）没补 tex_id 列写 → data 13 列 vs header 14 列 → 4 字节 skew，`ReadMesh` 把 idx_count 读成 vert_count，`ClipCount` 读越界。
**根因**：手搓 blob byte[] 的 C# 测 fixture 散落多文件多 builder（FrameBlobTests/FrameBlobV2Tests×2/MirrorPoolFlattenTests/MirrorPoolTests×2）；version bump 后 `ExpectedVersion` 变，所有产「应被接受」blob 的 builder 都要升，task review 只抽查漏了一个。
**解决**：bump version 时 grep 全 C# 测目录 `version=2u`/`HeaderLen = 88`/`elemSize = {`（13 项）/`13 \* 4`，逐 builder 升（version/HeaderLen/offs 数组/elemSize 项数/loop 边界/补 tex_id 列写）；Rust 侧 `num_col_offsets=columns.len()` 自动传播，C# arena offset 基准 `12+14*4` 也要改。
**教训**：blob 是 Rust↔C# 字节契约，version bump = 全仓 fixture 同步事件；靠 grep 枚举所有 builder，不能只改抽查的。reviewer 字节级核每个 builder 的 header 列数==data 列数。

### 坑 18：`using System;` 引入 System.Object，裸 `Object` 与 UnityEngine.Object 歧义（v1b.2）
**症状**：LoomStage.cs 加 `using System;`（为 `Encoding`/`Exception`）后，6 处裸 `Object.Destroy`/`DestroyImmediate` 编译报 CS0104 `'Object' is an ambiguous reference between 'UnityEngine.Object' and 'object'`。
**根因**：`System.Object`（C# `object` 关键字的类型）与 `UnityEngine.Object` 同名；两个 using 都在时裸 `Object` 二义。
**解决**：全限定 `UnityEngine.Object.Destroy/DestroyImmediate`（不动 using——`System` 还要 `Array.Empty`/`Encoding`）。MirrorPool.cs 无 `using System;` 故裸 `Object` 无歧义。
**教训**：Unity C# 文件若同时 `using System;`+`using UnityEngine;`，裸 `Object` 必歧义——直接全限定 `UnityEngine.Object`。

### 坑 19：BitConverter.GetBytes 无数组 overload，手搓 blob 索引数组写法错（v1b.2）
**症状**：C# 测 `BitConverter.GetBytes(new uint[]{0,1,2,0,2,3})` 编译报 CS1503 cannot convert uint[] to bool。
**根因**：`BitConverter.GetBytes` 只接标量（无 `uint[]` overload），最近重载是 `GetBytes(bool)`，编译器把 uint[] 当 bool。
**解决**：逐个 `GetBytes(0u)`/`GetBytes(1u)`...（对齐 MirrorPoolFlattenTests 已验证写法）；或循环。
**教训**：手搓 blob byte[] 的 C# 测，索引/数据数组一律逐元素 `GetBytes`，禁数组语法。

### 坑 20：打包器磁盘 atlas 文件名 ≠ .pkg.bin header 记的名 → Unity 找不到 atlas（v1b.3）
**症状**：PlayMode atlas sprite 全白占位（atlas.png 没载），但 .pkg.bin 解析正常、Console 无报错。
**根因**：main.rs 用 `out_path.with_extension("atlas.png")` 写磁盘 → `<stem>.pkg.atlas.png`；lib.rs 把 `"loom.atlas.png"` 写进 AtlasSection header。Unity 按 header 名 `Path.Combine(StreamingAssets, atlas_filename)` 找 → 名不匹配 → `File.ReadAllBytes` 抛 → 跳过 → 白占位。
**解决**：main.rs 用 packer 返回的 `p.atlas_filename` 拼磁盘路径（`out_parent.join(&p.atlas_filename)`）→ header 与磁盘同串，by-construction 一致。
**教训**：打包器产两文件（.pkg.bin + atlas.png）时，**磁盘 atlas 名必须 == header 的 `atlas_filename`**（后端按 header 载）；用同一变量拼两端，别各算各的。

### 坑 21：删 pub API 只验单 crate → 依赖 crate 编译断裂（v1b.3）
**症状**：T1 删 `TextureRegistry::register` 只跑 `cargo test -p loomgui_core`（绿），但 `loomgui_ffi_c`（register_texture FFI 调 register）编译断裂，到 T3 跑 workspace 才发现。
**根因**：`register` 是 pub API，被 ffi_c 跨 crate 调用；单 crate 测不覆盖跨 crate 依赖。
**解决**：删 pub API 必跑 `cargo test --workspace`（或至少 `cargo build` 所有依赖 crate）；T1 acceptance 只写 `-p loomgui_core` 是 plan 验测范围写窄。
**教训**：删/改 pub API（尤其被 FFI 跨 crate 用）→ acceptance 必含 `cargo test --workspace`，单 crate 绿 ≠ workspace 绿。

### 坑 22：fgui DoFairyBatching 不合并 mesh——照搬不够，core 要显式合并（v1b.4）
**症状**：初版 v1b.4 设计以为「照搬 fgui DoFairyBatching = N→1 draw call」。
**根因**：读 fgui Unity 源码（NGraphics.cs）发现每元素独立 MeshFilter+MeshRenderer，零 Graphics.DrawMesh/跨元素顶点拼接——DoFairyBatching 只重排 sortingOrder 让同 material 相邻，靠 **Unity Dynamic Batching 隐式合**（不可控、URP 下与 SRP Batcher 互斥、顶点≤300 限）。
**解决**：LoomGUI core 显式合并（render::merge::merge_meshes）——补 fgui 靠 Unity 隐式做的那步，确定性 N→1、跨后端。
**教训**：调研参考引擎时读源码确认「它实际做什么」vs「文档/印象说做什么」——fgui「合批」≠「合并 mesh」。

### 坑 23：fgui AABB 相交语义——同 material 相交仍聚拢保相对序（v1b.4）
**症状**：brief 测 `reorder_unit_overlapping_keeps_order` 断言 `[0,1,2]`（相交保序不动），实际 `[0,2,1]`。
**根因**：fgui 算法（Container.cs:923-931）相交 break 时用**已算出的 k**（同 material 聚拢点）——同 material 相交仍前移紧邻（不越目标，保绘制序 A→C）；不同 material 相交才不越过（k=m=i 不动）。「相交保序」=保相对绘制序非「完全不动」。
**解决**：断言改 `[0,2,1]` + 注释澄清。同 material 相交合并视觉安全（index buffer 保相对序）。
**教训**：算法移植按源码逐行 trace 验，勿按文字描述想当然；同 material 相交也能合（利好合批率）。

### 坑 24：merged node_id 必须=锚（batch 内 min）否则动画 GO 抖动（v1b.4）
**症状**：merge 后节点身份若每帧变（batch 划分随动画变），MirrorPool `_pool[node_id]→GO` 频繁增删 GO。
**根因**：merged 节点「虚拟」（多原始节点合并），无稳定 node_id → batch 划分变 → node_id 变 → GO 抖动。fgui 无此问题（不 merge，每元素稳定 GO）。
**解决**：merged node_id=batch 内最小原始 node_id（锚）——batch 划分不变时锚稳定→GO 复用零抖动。MirrorPool 零改（按 node_id 复用，看不出 merged vs 单）。
**教训**：merge 改变节点结构，必须给 merged 节点稳定身份（锚），否则后端池复用抖动。

### 坑 25：既有测用 N 同级同 DrawState 节点，merge 后 index OOB（v1b.4）
**症状**：`build_assigns_monotonic_keys`（root>[a,b] 3 Container）merge 后 3→1 节点，`rns[1]`/`rns[2]` OOB。
**根因**：3 同级 Container 同 DrawState（tex=0,program=0,mask=0）→ merge 成 1。
**解决**：改嵌套 clip 链（root clip→mid clip→leaf，3 不同 mask_context）→ 不同 DrawState → 不 merge → 保 3 节点，原 intent 保留。
**教训**：merge 上线后，既有「多同级同 DrawState 节点」测会 OOB——改成不同 DrawState（嵌套 clip/不同 tex_id）保留多节点。

### 坑 26：CJK sample width 放根节点 → root_size 覆盖致不换行（v1b.5）
**症状**：PlayMode CJK 段落整段挤一行不换行（~948px，远超 240px 约束）。
**根因**：sample 把 `width:240px` 放**根节点** div → `solve`（mod.rs:125 `set_style size=Length(root_size)`）强制覆盖根节点 size 为 root_size(1080) → Text 子 measure `known.width` 收 1080（非 240）→ 整段<1080 不换。§2.5 根节点 measure 陷阱的 sample 实例。
**解决**：sample 包一层 `.root` Container（吃 root_size）+ `align-items:flex-start`（防子项 cross 轴拉伸），`.c` 文本 div 作子节点（width:240 不被覆盖）。独立 layout 验 Text rect=240x113（逐字断行）。
**教训**：sample/测的**受约束文本/图必须在根节点之下**，根节点 size 必被 root_size 覆盖——根节点只作 viewport 容器，不设显式 CSS size。

### 坑 27：mandatory break 留 `\n` 字面量 → 下游幽灵 `.notdef` 字形（v1b.5）
**症状**：含 `\n` 文本换行时，行 text 含字面 `\n` → glyph gen 给 `\n` 产 `.notdef`（GlyphId 0）幽灵字形进 blob text_arena。
**根因**：unicode-linebreak 的 mandatory break 在 `\n` 字节 offset 断，segment `content[..offset]` 含 `\n`；flush 行时未 strip。
**解决**（defer post-merge，M3）：mandatory flush 前 `cur.trim_end_matches(|c| c=='\n'||c=='\r')` + 回归测断行 glyph 数。**当前不阻塞**：v1b.5 sample 无 `\n`；Unity `TextRasterizer.cs:67` `GetCharacterInfo('\n')`=false→continue 静默跳过幽灵字形，**无视觉伪影**（Rust 侧冗余，渲染干净）。
**教训**：断行产出喂 glyph gen 前须净化控制符；但后端 `GetCharacterInfo` false→continue 是天然兜底（坑 14 codepoint 路径副产物）。

### 坑 28：Unity 新旧输入系统不匹配 → InvalidOperationException（v1c.1 PlayMode）
**症状**：PlayMode 每帧 `InvalidOperationException: You are trying to read Input using UnityEngine.Input class, but you have switched active input handling to Input System package`。
**根因**：plan §7 选旧 `UnityEngine.Input`（桌面 Mono 最简），但工程装了 Input System package 1.19 且 Player Settings Active Input Handling≠Old → 旧 API 运行时禁用。
**解决**：`LoomInputCollector` 双路径 `#if ENABLE_INPUT_SYSTEM`（`Mouse.current` API）+ asmdef 加 `"Unity.InputSystem"` reference + Player Settings 改 Both/New（§3.10）。
**教训**：Unity 输入系统是工程级配置，plan 选型须先查工程 `ProjectSettings.asset activeInputHandler` + manifest 是否装 InputSystem package；默认走双路径兼容最稳。

### 坑 29：hover/active 只设命中点自身 → 文字子挡父 hover（v1c.1 PlayMode）
**症状**：hover 按钮的**文字区（上半段）不变蓝**，下半段（非文字）变蓝；click 文字区也无响应。
**根因**：v1c.1 `hover_diff` 只设 `cur_hit.hovered=true`（单点），父不因子孙 hover 而 hover。但 CSS `:hover` 语义是「鼠标在元素**或后代**上」+ fgui `HandleRollOver` 维护 `rollOverChain` 沿祖先链（`element=element.parent`）给 target+所有祖先派 RollOver/Out。命中 Text 子 → 只 Text.hovered，btn.hovered=false → `.btn:hover` 不匹配。
**解决**：`set_hovered_chain`/`set_active_chain` 沿 parent 链设 target+所有祖先 hovered/active；事件派发仍单点（v1c.1 无冒泡，v1c.2 加 BubbleEvent）。
**教训**：伪类状态须沿祖先链（对齐 fgui rollOverChain + CSS 祖先语义），单点设导致子孙挡父；「子节点挡父 hover/click」是 UI 框架经典坑，对照 fgui rollOverChain 设计。

### 坑 30：自动 Text 子节点不消费 StyleSheet（v1c.1 defer 根因 a）
**症状**：`span{pointer-events:none}` 写进 CSS，但 `<div>文字</div>` 自动建的 Text 子 `touchable` 仍 true（CSS 没匹配到它）。
**根因**：`build_scene` 给 Container 裸文本自动建的 Text 子**不是 DOM 元素**——`resolve_styles` 跑在 build_scene 前只算 DOM 元素，自动 Text 子拿不到任何 CSS 规则。只有显式 `<span>` 走 DOM resolve 才吃到 CSS。
**解决**（defer v1c.x）：修坑 29（hover 祖先链）后影响降级——文字挡命中但父也 hover，故不需 pointer-events 穿透。根治须 build_scene 后给自动 Text 子补 resolve（架构改），或框架默认 Text `touchable=false`。v1c.1 sample 用显式 `<span>` 绕（修坑 29 后已回归裸文本）。
**教训**：自动建的节点（非 DOM 元素）不消费 StyleSheet——CSS 规则只作用于 parse 期 DOM 元素；给自动子样式化须显式标签或框架默认。

### 坑 31：brief 测断言过强——`hover_chain_idempotent` assert `out.is_empty()` 但 Move 每次 emit（v1c.2）

**症状**：v1c.2 hover_diff 祖先链 diff 的幂等测验 `out.is_empty()`（同点 Move 第二次），但 v1c.1 Move handler 命中即产 `EVT_MOVE`（§7.1「move 恒产」），故 out 非空，测 FAIL。
**根因**：测的本意是「hover diff 幂等」（同点无 RollOver/Out），不是「无任何事件」。Move 事件每次 emit（指针移动事件，非 hover 变化），照 fgui `onTouchMove` 每次 dispatch。
**解决**：测断言改为 `out.iter().all(|e| e.event_type != EVT_ROLL_OVER && e.event_type != EVT_ROLL_OUT)`（无 hover 事件，Move 允许）。
**教训**：写测断言要精确反映验的语义——「幂等」≠「无事件」。Move/scroll 等每次产的事件不能和 hover/diff 状态变化混在 `is_empty` 断言里。

### 坑 32：implementer 为让 brief 测通过改实现（超 scope 破坏语义）（v1c.2）

**症状**：T1 implementer 为让坑 31 的 brief 测 `out.is_empty()` 通过，改 Move handler 加 `ancestor_chain 不变时抑制 EVT_MOVE`——超 brief scope + 破坏 §7.1 恒产 + 会破坏 v1d drag（onTouchMove 驱动，同节点内移动也要收）。
**根因**：implementer 把 brief 测当权威，brief 测与既有语义冲突时**改实现适配测**（方向反了）。
**解决**：fix 恢复 Move 无条件 emit + 改测断言（坑 31）。
**教训**：brief 测 vs 既有语义冲突时，**改测不改实现**（除非 plan 明确要求改语义）。implementer 按 brief verbatim 转录遇测-实现冲突应 flag DONE_WITH_CONCERNS 让 controller adjudicate，而非自作主张改实现。controller review 要验「超 brief scope 的改动」。

### 坑 33：C# EventBridge internal 测跨 namespace 不可见（v1c.2）

**症状**：T3 `EventBridge`（internal）单测在 `LoomGUI.Tests` namespace 直接 `new EventBridge()`——跨 namespace 不可见，编译报 inaccessible。
**根因**：v1c.1 测只碰 public 类型，无此需求；v1c.2 EventBridge internal + 测直接构造触发。
**解决**：`EventBridge` internal → public（测访问 + 避 `InternalsVisibleTo` 配置；EventBridge public 无害，业务通过 AddListener 用不直接 new）。
**教训**：C# 类型可见性要考虑测访问——单测直接构造的类型须 public 或加 `[InternalsVisibleTo("Tests")]`（项目已有此机制，坑 13 同源 csbindgen InternalsVisibleTo）。

## 6. 调试/验证技巧

- **★ 实现 v1+ 后端/渲染/对象模型前，先参考 `temp/FairyGUI-unity/` 源码**（对照机制、避免走歪——本 session 因没先看 fgui 的 sortingOrder/rect-mask/MaterialManager，初版设计走了弯路：误用 z 排序、误以为 rect mask 要独立 GO、把绘制序想复杂）。
- `cargo test -p loomgui_core`：全量（v0 ~52 测试）。
- **feature gate 构建矩阵**（v1b.1）：`cargo build -p loomgui_core --no-default-features` + `-p loomgui_ffi_c --no-default-features` 皆编（证 runtime 可无 parser）；`cargo build -p loomgui_pkg`（带 parse）。
- **打包器冒烟**（v1b.1）：`cargo run -p loomgui_pkg -- in.html in.css -o out.pkg.bin -w 1080 -h 1920`；前 4 字节 `4c 50 4b 47`="LPKG"。
- `cargo run --example v0_snapshot`：端到端产 `v0_snapshot.json`。
- insta 快照：`INSTA_UPDATE=always cargo test --test snapshot` 首次接受，再裸跑锁定。
- 字体路径：`format!("{}/tests/fixtures/DejaVuSans.ttf", env!("CARGO_MANIFEST_DIR"))`。
- 改 `ResolvedStyle` 默认/映射后，跑 layout + snapshot 测试看布局变化。
- taffy 布局调试：看 `Node.layout_rect`（solve 回写的绝对坐标）。
- 查 crate 实际 API：`~/.cargo/registry/src/<crate>-<ver>/src/`。
- **PlayMode 命中诊断**（v1c.1）：Unity 侧加 diag log——`LoomInputCollector.Collect` Down 时 log `design=(dx,dy)` + `LoomStage.LateUpdate` log `mouse/screen/evLen/onUI`。core 侧独立验：写临时 `examples/dump_xxx.rs` 跑 `Stage::tick_and_render` 后 `hit_test(scene, design_pt)`。**core 命中但 PlayMode onUI=false → 坐标映射或 set_input 传输问题**（如输入系统不匹配坑 28）；**core 也不命中 → AABB/坐标换算**。例：坑 29 诊断时 core `hit_test(270,46)→Some(btn1)` 但 PlayMode onUI=false，定位到 Collect 未被调（LoomInputCollector 组件没挂 GO）。
- **命中 y 偏移诊断**（v1c.1）：「按钮下半段响应上半段不响应」= Text 子节点 AABB 盖父上半段（`<div>文字</div>` 自动 Text 子 `layout_rect` 与父重叠上半）→ 逆等效序命中 Text 而非父。dump 节点 AABB 确认子是否盖父 + 对照坑 29（hover 祖先链）根治。
- Rust→Unity 闭环：改 Rust 后 `cargo build -p loomgui_ffi_c --release` → 关 Unity → `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/`。
- Unity 验证：Test Runner EditMode（`Window→General→Test Runner`）；PlayMode 看 Game 视图渲染；PlayMode 前确认 `.dll` 是最新版。
- 跨语言 round-trip：Rust `build_blob` ↔ C# `FrameBlob` 靠手搓 blob byte[] 的 EditMode 测互验（blob 布局是 Rust↔C# 契约，两端须字节级一致；改列/偏移必同步）。**手搓多节点 fixture 必 SOA 列优先**（坑 12，单节点掩盖 AoS 错）。
- **bump blob version 清单**（v1b.2，坑 17）：① Rust `blob.rs` VERSION+COLUMNS+`num_col_offsets=columns.len()`（自动传播 header_len）；② C# `FrameBlob.cs` `ExpectedVersion` + 所有 arena offset 基准（`12+14*4` 非 `13*4`）；③ grep C# 测目录**所有** builder：`version=Nu`/`HeaderLen`/`elemSize = {`/`i < N`/末列写——逐个升，header 列数==data 列数；④ 重编+关 Unity 换 .dll。
- **stale .dll 诊断**（v1b.2）：PlayMode 全不渲 + Console 干净 → `md5sum` 对比 fresh release .dll vs `Assets/Plugins/LoomGUI/`，committed .dll 应==release（坑 10）；committed .dll md5 记在 progress ledger 便于核对。
- **stale .dll 诊断**：PlayMode **全不渲 + Console 干净** → `md5sum target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`，不等 = stale（Rust 改 blob/ABI 格式没换 .dll，坑 10）。
- **T7 perf 基线**：500 节点静态 ~5-8ms/帧（120-200fps，无卡顿过 §9.3）。成本 = 朴素每帧全量重传（Rust 没 dirty/Unchanged 跳过 + `ReadMesh` per-frame alloc 数组）；优化（dirty 跳静态≈0 + ArrayPool 冷帧≤2ms）归 v1e。
- PlayMode 验前 checklist：① Rust 改过 → 重编+换 .dll（关 Unity）② LoomStage `_font` 赋值 ③ Console 看红字 ④ Hierarchy 看 GO 不累积。
- **删 pub API 必验 workspace**（v1b.3，坑 21）：`cargo test -p <crate>` 绿 ≠ workspace 绿——pub API 被 FFI/其他 crate 跨 crate 用时单 crate 测不覆盖。acceptance 写 `cargo test --workspace`。
- **打包器两文件名一致**（v1b.3，坑 20）：packer 产 .pkg.bin + atlas.png，验磁盘 atlas 名 == .pkg.bin header `atlas_filename`（后端按 header 载）；各算各的 → 后端找不到 → 静默白占位。
- **atlas batching 验收**（v1b.3）：FrameDebugger 看 atlas sprite 是否**同 Material**（底线）；draw call↓ 需开 URP Dynamic Batching（best-effort，mesh 不合并不保证 N→1）。
- **重打 sample 流程**（v1b.3）：改 sample html/css/PNG 后 `cargo run -p loomgui_pkg -- samples/atlas/page.html samples/atlas/page.css -o StreamingAssets/loom_atlas.pkg.bin`（自动写 atlas.png 旁挂）；Unity 开着会 reimport，重进 PlayMode 重跑 Awake 载新包。
- **blob 零改验证**（v1b.4，spec §9）：merge 改 build_render_nodes 输出但 blob/MirrorPool 必零改。验证：① `git diff --stat` 确认 blob.rs 改动全在 `mod tests`（生产 build_blob body 零行 diff）；② blob round-trip 测（TestView 逐顶点读 `mesh_vert`/`mesh_color_alpha`）验 merged transform=0→re-base 减 0=绝对 verts + alpha=1→×1=不二次烤。
- **merge 两路径一致**（v1b.4）：黄金等价测（inline==pkg）验 merge 对两路径同构——FAIL=inline/pkg scene 不一致（真实 bug 非测问题）。
- **merge PlayMode 验收**（v1b.4）：FrameDebugger draw call 数（连续同 atlas 段理想 1）+ Hierarchy loom_node GO 数（=batch 数<<节点数）+ 无花屏（证 merge 正确性，index buffer 保相对序）。
- **CJK PlayMode 验收**（v1b.5）：窄宽（240px）中文段落应**逐字换行多行**（每行~10 CJK 字宽），English 按词不拆，CJK 标点正常，**无 tofu 方块**（tofu=字体未载/`_font` 未配）。判对错看 3 点（字对/换行了/布局正常）非精确断行位置（baseline 现状占位，design §9.1 实现期对照 Chrome 调，defer）。配置：LoomStage `_usePackage=true`+`_pkgFile=loom_cjk.pkg.bin`+`_fontFile=wqy-microhei.ttc`+Inspector `_font`=CJK Font。
- **CJK 字体获取 fallback**（v1b.5）：brief 列的字体下载源常全 404 → 用 **GitHub tree API** 找 repo 内实际 .ttc 路径（`https://api.github.com/repos/<owner>/<repo>/git/trees/<branch>?recursive=1` grep `.ttc`）。文泉驿微米黑 live 源：`chai2010/wqy-microhei-go/data/wqy-microhei-0.2.0-beta/wqy-microhei.ttc`。
- **断行 layout 独立验证**（v1b.5）：sample 不换行时写临时 `examples/` 验 `solve` 后 Text 节点 `layout_rect.w`——应=约束宽（240）非 root_size。区分「断行逻辑坏」vs「max_width 喂错」（sample 根节点 measure 覆盖，坑 26）。
- **.dll 被 Unity 锁挡 merge**（v1b.5）：ff-merge 报 `unable to unlink .dll: Invalid argument` + working tree dirty .dll → Unity 开着锁 native .dll（坑 10 同源）。解：关 Unity，或 `git checkout HEAD -- <dll>` 还原 working tree 到 main 版本再 merge（merge 带 v1b.5 新 .dll 过来）。pre-existing 脏 .asset（DefaultVolumeProfile/ProjectSettings）`git stash push --` 单独隔离。

## 7. 已知问题/未完成（v0 ledger）

**v0 占位 → v1.x 优化**：
- mask_context id = counter+1 不稳定（节点增删抖动）。
- sort_key 无 FairyBatching AABB 重排（v0 保序）。
- ~~断行贪心非 UAX#14（CJK kinsoku 留 v1.x）~~（v1b.5 已落地：unicode-linebreak UAX#14，CJK 逐字断；kinsoku 标点禁则仍 defer）。
- baseline 未对 Chrome 校准（§9.1 实现期调）。
- Font 用 `Box::leak` 不释放（多字体 v1 换 owned）。
- ~~tex_id 16 位 hash 碰撞~~（v1b.2 已消除：registry 分配单调 tex_id，无 hash）。
- grayed 恒 false / BlendMode 仅 Normal。
- CSS order 排序跳过（taffy 0.5 无 order 字段，DOM 序）。
- border_width 仅取 top（非均匀 border）。
- opacity % 语义（`50%`→50.0 非 0.5，brief 原行为）。

**v1a Phase 1 ✅ 完成（merged main @ 7920bbd）**：FFI crate（loomgui_ffi_c：csbindgen + SOA blob）+ Unity 6.5 URP 后端镜像（FrameBlob/MirrorPool/MaterialManager/LoomStage/shader）。静态色块在 Unity Game 视图真渲染——**v1 最大风险缝闭合**。spec `docs/superpowers/specs/2026-06-19-v1a-unity-render-design.md`、plan `docs/superpowers/plans/2026-06-19-v1a-unity-render-phase1.md`。

**v1a Phase 2 ✅ 完成（merged main @ 9889afa）**：Text（Rust 笔位+Unity 光栅+textureRebuilt，§2.10）+ rect mask（嵌套交集+_ClipBox，§2.11）+ 500 节点压测（buffer 复用）+ Domain reload（ResetStatics 接 shutdown）。**v0 fixture（div+文本+img+rect mask）在 Unity 真渲，500 节点静态无卡顿，进出 Play 不 crash**。spec `docs/superpowers/specs/2026-06-19-v1a-unity-render-phase2-design.md`、plan `docs/superpowers/plans/2026-06-19-v1a-unity-render-phase2.md`。踩坑：.dll 重编换（坑 10）、ExecuteAlways GO 泄漏（坑 11）、SOA fixture（坑 12）、codepoint（坑 14）。

**v1a Phase 2 defer → v1e**（perf，spec §4.5 / §7）：
- **静态帧朴素全量重传**：Rust 没做 dirty/`Unchanged` 跳过（每帧对所有节点 emit Mesh）→ MirrorPool 每帧重传全部。优化：Rust dirty 跟踪 emit Unchanged → 静态帧≈0。T7 基线 ~5-8ms/500 节点。
- `FrameBlob.ReadMesh` 仍 per-frame alloc `MeshSegment` 数组（UploadMesh List 复用 T7 已做，ReadMesh 没做）→ ArrayPool 化。
- `TextRasterizer.BuildMesh` per-rebuild alloc 4 List（text-heavy 场景才痛，T7 未触）。
- shader 非 CLIPPED 路径无条件算 clipPos（fgui `#ifdef` 守卫）；ArrayPool 帧拷贝、冷帧/换页帧 FFI ≤2ms。
- Font `Box::leak`（真进程级泄漏，~700KB/Stage）缓存化——×20 域重载测**未现显著增长**，按 <5MB 阈值**推 v1e**（非阻塞）。
- 坐标 nesting+父相对（transform 继承/事件）→ v1c。

**v1b.1 ✅ 完成（merged main @ 5706a7b）**：打包器 `loomgui_pkg` CLI + `.pkg.bin` v1 格式（Rust-internal，§2.12）+ `Stage::load_package` + `Scene::build` 共享建树 + parse feature gate（runtime 可无 parser）+ FFI `loomgui_stage_load_package` + Unity `_usePackage` 接线。**包路径渲染 == inline 渲染（黄金等价），PlayMode 验过**——验收 #6 达成。spec `docs/superpowers/specs/2026-06-20-v1b-packager-design.md`、plan `docs/superpowers/plans/2026-06-20-v1b-packager.md`。踩坑：magic 撞 frame blob（坑 15）。

**v1b 拆分**：A 打包器/二进制包/加载器（v1b.1 ✅）、B 真纹理加载（v1b.2 ✅）、C 图集打包（v1b.3 ✅）、mesh 合并（v1b.4 ✅）、**D 文本 CJK（v1b.5 ✅，最小 CJK A 档）**。v1b 全收尾。各自 spec。

**v1b.2 ✅ 完成（merged main @ 691835a）**：真纹理加载——core `TextureRegistry`（src→TexMeta{tex_id,w,h}，§2.13）+ render/measure 三档（CSS>真实>64）+ blob v3（14 列加 tex_id）+ FFI `register_texture`/`image_src_count`/`image_src_at`（collect）+ C# FrameBlob v3 + MirrorPool 按 tex_id 绑材质 + LoomStage collect→load PNG→register→`_texMap`。**`<img src>` 渲真像素（PlayMode 验），命中 G7**。spec `docs/superpowers/specs/2026-06-20-v1b-texture-design.md`、plan `docs/superpowers/plans/2026-06-20-v1b-texture.md`、progress `.superpowers/sdd/progress.md`。踩坑：FFI 无尾 NUL 串契约（坑 16）、blob version bump 级联 C# fixture（坑 17）、`using System;` Object 歧义（坑 18）、BitConverter 无数组 overload（坑 19）。

**v1b.2 defer**：~~图集（TextureView UV region）~~（v1b.3 ✅，refcount 仍 defer）、alpha 纹理、多 Stage 全局 registry、Addressables/YooAsset 异步、移动端 StreamingAssets（UnityWebRequest）、inline 路径真纹理、NPOT/压缩/mipmap。

**v1b.3 ✅ 完成（merged main @ 25171f8）**：图集打包——打包器 `image` crate + shelf（§3.8）→ atlas.png + AtlasSprite 表进 .pkg.bin **v2**（§2.12）+ core `TexMeta`+uv/`build_registry`（§2.14）+ render quad 按 region 烤 UV + FFI `atlas_count`/`atlas_info` 取代 v1b.2 loose collect/register + Unity `LoadAtlas`（1 atlas Texture2D 共享）。**blob v3 不变**（per-vertex UV 已在）。**PlayMode 验**：3 图共显方块 + FrameDebugger 同 Material（atlas 批合条件具备）。诚实认知：mesh 不合并→不保证 N→1 draw call（SRP Batcher + Dynamic Batching）。spec `docs/superpowers/specs/2026-06-21-v1b-atlas-design.md`、plan `docs/superpowers/plans/2026-06-21-v1b-atlas.md`、progress `.superpowers/sdd/progress.md`。踩坑：磁盘/header atlas 名不一致（坑 20）、删 pub API 漏验 workspace（坑 21）。

**v1b.3 defer**：rotation/trim（UV 修正 §8.2）、多图集（sprite 带 atlas_idx）、refcount/on_release（§12.4）、~~mesh 合并~~（v1b.4 ✅）、POT/压缩/mipmap。

**v1b.4 ✅ 完成（merged main @ a09bb0b）**：mesh 合并 + AABB 保序重排——core `reorder_for_batching`（batch.rs，fgui DoFairyBatching core 化）+ `merge_meshes`（merge.rs，连续同 DrawState Mesh→单 merged payload）。**blob v3/MirrorPool 零改**（merged transform=0/alpha=1 让 re-base+alpha 烤对 merged 无效，spec §9）。锚 node_id（min batch）解动画 GO 抖动。**PlayMode 验**：3 连续同 atlas sprite→1 draw call + GO 数=batch 数 + 无花屏。重要认知修正：fgui 不合并 mesh（靠 Unity Dynamic Batching 隐式），LoomGUI core 显式合并补它没做的。spec `docs/superpowers/specs/2026-06-21-v1b-mesh-merge-design.md`、plan `docs/superpowers/plans/2026-06-21-v1b-mesh-merge.md`。踩坑：fgui 不合并（坑 22）、AABB 相交语义（坑 23）、锚 node_id（坑 24）、既有测 OOB（坑 25）。

**v1b.4 defer**：动画 opt-out merge、增量 dirty merge/diff（同 v1e perf）、同字体 Text 合并（后端改）、blend 进 DrawState key、AABB 高级优化（sweep-and-prune）、GPU instancing（v2 自建 renderer）。

**v1b.5 ✅ 完成（merged main @ 7b24516）**：CJK 文本最小可行（A 档）——启用 Cargo.toml:11 已有 dead dep `unicode-linebreak` 重写 `measure_text` 断行（`split(' ')`→UAX#14 greedy fill，CJK 逐字断修核心 bug）+ CJK 字体 fixture（文泉驿微米黑 .ttc index 0）+ Unity `_fontFile` 字段（默认 DejaVu 不破坏现有）+ CJK sample + TextRasterizer CJK EditMode 测。**blob/MirrorPool/TextRasterizer/shader 零改**（CJK codepoint ≤BMP 走同一光栅路径）。**PlayMode 验**：240px 窄宽中文段落逐字换行 8 行 + English 按词 + CJK 标点 + 无 tofu。兑 design §9.1「CJK+ASCII+CJK 标点」v1 承诺。对照 fgui BuildLines 后维持 unicode-linebreak（对齐 Chrome/AI 可预测性/design 契约），fgui `wordLen<20`+`toMoveChars=1` 作超长词边界参考。spec `docs/superpowers/specs/2026-06-21-v1b-cjk-text-design.md`、plan `docs/superpowers/plans/2026-06-21-v1b-cjk-text.md`。踩坑：unicode-linebreak API 不符草稿（§3.9）、CJK sample 根节点 measure 覆盖（坑 26）、mandatory `\n` 幽灵字形（坑 27）。

**v1b.5 defer**：font fallback 链、多 font-family、per-glyph font_id、TextLayout runs 三表投 blob、emoji/组合符号 shaping/RTL、kinsoku 标点禁则、measure 缓存、line-height/baseline 校准、Font `Box::leak` 缓存化（v1e）、M3 mandatory `\n` 净化（post-merge follow-up，Unity GetCharacterInfo 静默跳过故无视觉伪影）。

**v1c.1 ✅ 完成（merged main @ 44e715c）**：事件/命中/输入最小交互闭环——`input.rs`（PointerEvent/EventRecord + PointerState 单指针状态机 + hover/active **祖先链**）+ `hit.rs`（逆等效绘制序命中，layout_rect AABB+clip+pointer-events+disabled 仍命中）+ `style/dynamic.rs`（伪类重匹配，全量重 cascade §5.3；selector 类型从 parse 迁此修 parse-gate）+ pkg.bin v2→v3（DynamicRuleSection bincode + NodeBlock classes/id_attr）+ FFI 4 函数（pull 模式绕 IL2CPP 回调）+ Unity `LoomInputCollector`/`LoomEventHandler`（listener 在 C#）。命中验收 #3（hover/active 反馈）/#5（is_pointer_on_ui）。spec `docs/superpowers/specs/2026-06-21-v1c.1-event-hit-input-design.md`、plan `docs/superpowers/plans/2026-06-21-v1c.1-event-hit-input.md`、progress `.superpowers/sdd/progress.md`。**PlayMode 验收修 3 真实坑**：Unity 新旧输入系统不匹配（坑 28，双路径 `#if ENABLE_INPUT_SYSTEM`）、hover/active 只设命中点无祖先链（坑 29，对齐 fgui rollOverChain 修）、自动 Text 子不消费 StyleSheet（坑 30，defer 根因 a）。subagent-driven 11 task 全 Approved，3 次 API 限流中断无质量损失（T3 半成品 discard 重派，T9/T11 产出完整只重派 review）。`id_attr` 偏离（`Node.id: NodeId` 占用，合理）。

**v1c.1 defer**：根因 a（自动 Text 子消费 StyleSheet，架构改，修坑 29 后降级）、~~事件冒泡 BubbleEvent（v1c.2）~~（v1c.2 ✅）、多触摸 capture（v1c.3）、invalidation set 伪类重匹配优化（v1e 撞墙）、transform world_to_local 命中（v1d）、滚轮/键盘/IME 输入（v1d+/G5）。

**v1c.2 ✅ 完成（v1c.2 branch，待家里机验）**：事件路由完整化（方向 A）——核心 `hover_diff` 祖先链 diff（点1，修坑 29 嵌套多发，`last_hovered_chain`+`ancestor_chain`）+ FFI `node_parent`（C# 路由沿链，sentinel 0xFFFFFFFF）+ C# `LoomEventHandler` 重写（bubble/capture 两阶段照 fgui BubbleEvent + stop + EventContext 对象池 + 多 callback + 委托 remove + RollOver/Out 直派）+ 主设计 §10.2/§6.3/§15 修订（路由降级业务侧，删 `Node.listeners`）。**EventRecord/event blob/frame blob/.pkg.bin/MirrorPool/shader 零改**。spec `docs/superpowers/specs/2026-06-23-v1c.2-event-bubbling-design.md`、plan `docs/superpowers/plans/2026-06-23-v1c.2-event-bubbling.md`、验收文档 `docs/v1c.2-home-verification.md`。**两机约束**：core cargo test 本机 164 测全绿；C# 本机写未编译（无 Unity），家里机 EditMode（4 路由测骨架补 handle）+ PlayMode 5 条待验。subagent-driven 6 task 全 Approved + final review Ready。踩坑：brief 测断言过强（坑 31）、implementer 改实现适配测超 scope（坑 32）、EventBridge internal 测不可见（坑 33）。

**v1c.2 defer**：CaptureTouch/多触摸槽（v1c.3）、click downTargets 链兜底+双击（v1c.3）、stopImmediatePropagation、broadcast 子树广播（v1.x）、onKeyDown/Up/onMouseWheel 路由（v1d+）、AncestorChain 池化（Move 热路径，fgui 池 callChain，v1c.2 YAGNI 未池）、transform world_to_local 命中（v1d）。

**v1 其余 defer（v0 起，未动）**：
- v1b 全收尾（A/B/C/mesh/CJK ✅）+ v1c.1 最小交互闭环 ✅ + **v1c.2 路由完整化 ✅（待家里机验）**。
- **下一个**：v1c.3 多触摸 capture / v1d 动画滚动拖拽焦点（§11/§12.7，#2 可滚动容器）/ v1e perf。
- NativeHost/virtualization/shape mask：v1.x。

完整 defer 表见各 spec §7；v1a Phase 1 实现 ledger 见 `.git/sdd/progress.md`。

## 维护

每次 LoomGUI 开发/修复后，用 `session-summary` skill 把新踩坑/机制/调试技巧总结进本文件（§5 加坑、§3 加 API、§7 更 ledger）。本 skill 与代码一起提交。
