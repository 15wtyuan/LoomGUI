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
- `build_render_nodes(scene, font, &TextureRegistry)`：Container/Button→Mesh quad(背景色，全图 UV `[0,0],[1,1]`)，Image→Mesh quad（**tex_id 查注册表 + UV region**：v1b.3 按 atlas 子区 `uv_min/uv_max` 烤 4 角 UV，未注册=0 哨兵+全图 UV→白占位；v1b.2 前 v0 占位是 hash(src) 已删），Text→TextLayout 装 Text payload。`mesh::quad(rect,color,uv_min,uv_max)` 接 uv_rect（vert TL,TR,BR,BL ↔ sprite 角；v1b.2 全图即 0,0,1,1）。**v1-showcase 坑 64：Image 调用 swap v**（`[uv_min[0],uv_max[1]], [uv_max[0],uv_min[1]]`）——design y-down + LoomStage y-flip，TL 须映 texture 顶 (umin,vmax)；mesh::quad 本身不改（背景色块 UV 全图无方向）。
- `assign_sort_keys`：DFS 单计数器 sort_key，clip 的 Container 是 BatchingRoot 开新 mask_context。
- **v1b.4 AABB 保序重排 + mesh 合并**（§8.5）：`build_render_nodes` 末尾 `assign_sort_keys → reorder_for_batching → merge_meshes`。`reorder_for_batching`（batch.rs）= fgui `DoFairyBatching`（Container.cs:877-941）稳定插入排序 core 化——同 DrawState((texture,program,mask_context)) 不相交元素前移聚拢，相交保相对序（坑 23）；Text(program=1) batch break 不重排。`merge_meshes`（merge.rs）按 sort_key 扫连续同 DrawState Mesh→拼 merged payload。**锚 node_id**（merged=min batch，坑 24）解动画 GO 抖动；**merged transform=0/alpha=1** 让 blob.rs:70 re-base（减 0）+ blob.rs:90 alpha 烤（×1）对 merged 无效 → **blob/MirrorPool 零改**（§2.8 列结构不动，spec §9）；colors 只烤 alpha 分量（rgb 不动，color_tint 不传，坑 9）。

### 2.7 stage
- `Stage::new(font_path, root_size)` → `load_inline(html, css)` → `tick_and_render()` → `FrameData{nodes:Vec<RenderNode>, clips:Vec<ClipEntry>}`（v1a Phase 2：clips=嵌套交集后的 clip 表）→ `render_json()`。
- 静态首帧：tick 接空输入、dt=0。

### 2.8 FFI（loomgui_ffi_c，主文档 §14，v1a Phase 1）
- `extern "C"` 薄包装 + opaque `*mut StageHandle`；csbindgen 扫 `src/lib.rs` 生成 C# `Native` 类。
- ABI：`stage_new/free/load_html/tick/borrow_frame/shutdown`。string 走 UTF-8 `*const u8`+len；`borrow_frame(h, *mut usize) -> *const u8` 返 Rust 拥有的帧 blob（下 tick 失效；未 tick 返 null+len=0）。
- `StageHandle{ stage, frame_blob: Vec<u8> }`——tick 时 `build_blob` 覆写 frame_blob。
- `build_blob(&FrameData) -> Vec<u8>`（**v4**, version=4，v1d.3）：SOA 公共头 **18 列**（v3 的 14 + transform 列 `local_x/local_y`(2) → world matrix `m_a,m_b,m_c,m_d,m_tx,m_ty`(6)；列序 node_id@0,parent_id@1,visible@2,alpha@3,sort_key@4,mask_context@5,m_a@6..m_ty@11,payload_kind@12,mesh_off@13,mesh_len@14,text_off@15,text_len@16,tex_id@17）+ mesh arena + text_arena + clip 表。`num_col_offsets=columns.len()` 自动传播列数→header_len=12+18*4=84，arena Mesh@84/Text@92/Clip@100。magic+version 进 header，C# `FrameBlob.IsValid` 校验（防 stale v3 blob）。**mesh 顶点 re-base 两路径**（v1d.3 坑 49）：identity/merge 节点减 tx,ty→top-local；非纯平移节点不减（顶点已 box 本地）。全 LE。改 blob 格式必重编+换 .dll（坑 10）+ C# fixture 同步（坑 17）。
- v1b.3 FFI（常驻不 gate）：**删** v1b.2 的 `register_texture`/`image_src_count`/`image_src_at`（loose 散图模型被 atlas 取代）；**加** `atlas_count(h)->usize` / `atlas_info(h,i,*out_tex_id,*out_w,*out_h,*out_src_len)->*const u8`（返 atlas filename 无尾 NUL 串 + len，`*out_tex_id=(i+1)`，坑 16 len-based 读）。version 串 v1b.3。

### 2.9 Unity 后端（loomgui_unity，主文档 §14，v1a Phase 1）
- `FrameBlob`（BitConverter 解析 v2 blob，`IsValid` 校验 magic+version）→ `MirrorPool.Sync`（`Dictionary<uint,RenderObj>` O(n) stale-flag diff）。**flatten（Phase 2）**：所有 GO 挂**根**（非巢状——local_x/local_y 是绝对 design 坐标，巢状 SetParent 会双计父位置，坑见 §2.11/Phase 1 单节点未暴露），`localPosition=绝对`、`sortingOrder=sort_key`；kind=1 Mesh / kind=2 Text（→TextRasterizer）/ kind=0 跳过。**buffer 复用**：RenderObj 持可复用 List，`SetVertices(List)` 零 alloc（T7，500 节点压测）。
- `MaterialManager`：key=(program, texture, mask_context)——mask_context 进 key → 每 ctx 独立 Material 持各自 `_ClipBox`；ctx>0 → `EnableKeyword("CLIPPED")`（`#pragma multi_compile`）+ `SetClipBox`。**tint×alpha baked 进顶点色（Rust 侧）**，材质只带 texture+clip_box+blend。
- `LoomStage`（`[ExecuteAlways]` MonoBehaviour）：LateUpdate `tick→borrow_frame→Marshal.Copy→FrameBlob→MirrorPool.Sync`。根 GO `localScale=(sf,-sf,sf)`（shrink-to-fit sf=min(sw/dw,sh/dh) + y-flip 合一）+ `localPosition=(-sw/2,sh/2,0)`；UI 相机正交 `orthoSize=sh/2` `cullingMask=1<<6`(LoomUI) **独立于根**（不 SetParent）。shader `Cull Off`（根翻转 winding）。Phase 2：`[SerializeField] Font _font`（EnsureFont 兜底 AssetDatabase 加载 DejaVu）、`Font.textureRebuilt+=OnRebuilt`（OnDestroy 解绑）、`ResetStatics`（`SubsystemRegistration` 调 `loomgui_shutdown`+`TextRasterizer.ResetStatic`）、**Awake 清 root 下 loom_node 孤儿 GO**（ExecuteAlways 防累积，坑 11）。
- URP unlit shader：`col=tex2D×v.color`、`Cull Off`、`ZWrite Off`、`Blend[_Src][_Dst]` property、`CLIPPED` variant（rect mask `_ClipBox` discard，Phase 2 启用）。图片 v1a 占位 1×1 白贴图；**Text Phase 2 ✅**（font atlas）。**v1-showcase 坑 62/63（Linear 项目颜色管理）**：frag 手写 `SRGBToLinear`（vcolor.rgb——CSS sRGB 值在 Linear 项目不自动转 → 灰蒙蒙；URP Color.hlsl include 路径不稳故手写）+ `ALPHA_MASK` keyword（`#pragma multi_compile`，MaterialManager `program==1` text 启用）：text=`half4(vcol.rgb, vcol.a*tex.a)`（font atlas 是 alpha-mask，rgb 黑），image=`tex×vcol`（彩色 texture rgb）。

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
- **node 查询 enabler（v1c review）**：`Scene::find_by_id_attr`+`Stage::find_node_by_id`+FFI `loomgui_stage_find_node_by_id(h, byte* id, nuint len) -> u32`（无匹配→0xFFFFFFFF）+ C# `LoomStage.FindNodeById/SetNodeDisabled`——业务用 CSS id 定位节点（注册 listener / 设 disabled），替代硬编码 build 序 id（auto Text 子偏移不可靠，LoomInteractDemo 推断序 smell）。用既有 `Node.id_attr`（v1c.1 T3 NodeBlock 段，parser `id` 属性→el.id→Node.id_attr 全链路）。
- **两机约束（v1c.2 执行）**：core cargo test 本机 TDD 闭环；C# 代码本机写不编译（无 Unity），家里机 EditMode+PlayMode 验。改 Rust 后重编+commit .dll（坑10 两机变体）。subagent-driven 6 task 全 Approved。

### 2.17 多触摸 + CaptureTouch（v1c.3，§10.3）
- **多槽状态机**：`PointerState` 单指针 → `slots: Vec<TouchSlot>` 固定 5 槽（slot0=鼠标 `touch_id=-1` 常驻，slot1-4=触摸 `touch_id=-1` 空闲 Down 分配 fingerId）。照 fgui `TouchInfo[5]`。**鼠标+触摸同帧共存**（偏离 fgui 互斥——fgui 因 touchId=0 给鼠标撞 fingerId 才互斥，LoomGUI 鼠标 -1 绕开）。
- **EventRecord/PointerEvent 加 touch_id**（破 v1c.2 零改）：EventRecord 16→20B（+`touch_id:i32 @8`）；PointerEvent +`touch_id:i32 @4`（16B，**PointerKind 必须 `#[repr(u8)]`** 见坑 34）。touch_id 贯穿：PointerEvent@4 → EventRecord@8 → C# LoomEvent.touch_id → EventContext.touchId。
- **active/hovered 全局 union recompute**（删 v1c.1 `set_hovered_chain`/`set_active_chain`）：process 末尾 `recompute_hovered`（清所有→活跃槽命中链 union，任一指命中元素或祖先→`:hover`）+ `recompute_active`（清所有→所有 `is_down` 槽 **down_node** 命中链 union，基于 Down 时命中非当前 hit；**链遍历逐节点查 disabled 遇则截断**——坑 37，v1c review 补；hit 落非 disabled Text 子时 down_node 非 disabled，须沿链查 disabled 祖先）。修 v1c.2 单指 `set_active_chain(None)` 多指下互清 bug。RollOver/Out 仍 per-slot hover_diff（每槽独立链，EventRecord 带 touch_id）——**双语义**：RollOver/Out 描述「该指进出」，hovered 描述「任一指在其上」，可能不一致（A 移出 X 但 B 还在 X → X 收 RollOut 但 `:hover` 仍 true），各描述不同事实。
- **CaptureTouch/touch monitor（照 fgui）**：`EventContext.CaptureTouch()` 设 `_touchCapture` 标志，**消费即清零**（照 fgui EventDispatcher.cs:305-324）；C# `BubbleRoute` 的 capture 阶段 + bubble 阶段**各消费一次**记 `_captureNodeCap`/`_captureNodeBub`（cap/bub 各加一个 monitor，典型拖拽只 bubble 加 1）。Down 路由后 C# 调 `add_touch_monitor(touch_id, node)` FFI → 核心加进该槽 `touch_monitors`（去重，不实现 fgui -1 广播——fgui 自身不用）。Up 后核心清该槽 monitor。
- **Move 语义对齐 fgui**（v1c.2 行为变化）：核心 process Move 分支——**有 monitor 产 `Move@monitor` 直派（每 monitor 一条）；无 monitor 不产 Move 事件**（仍更新 last_pos+hover_diff）。C# `DispatchPending` Move 改 `DirectDispatch`（v1c.2 是 BubbleRoute）。fgui 鼠标 Move 无 monitor 也只发 Stage 自身（业务不注册=无回调）。**v1c.2 鼠标 Move 产事件沿链 bubble 的行为废止**——无现存业务破坏（interact sample 无 Move listener），要跟鼠标 Move 须 capture。
- **Up 去重**：核心产 monitor 的 Up 时若 `monitor==hit` 不重复产（避免同节点收两次 Up）。
- **FFI 加**（常驻不 gate）：`loomgui_stage_add_touch_monitor(h, touch_id:i32, node_id:u32)` + `loomgui_stage_remove_touch_monitor(h, node_id:u32)`（remove 用 `Vec::retain` 非 fgui null-sentinel，Rust 更干净）。version v1c.3。
- **Unity 采集**（`LoomInputCollector.Collect`）：每帧鼠标（touch_id=-1）+ 所有活跃触摸（fingerId）批量 set_input。新输入系统 `Touchscreen.current.touches` + `Mouse.current`；旧 `Input.touches`。Stationary 跳过，Began→Down/Ended+Canceled→Up。**set_input FFI 是 `PointerEvent*` 非 managed array**（csbindgen），须 `fixed`-pin（坑 36）。
- **click 沿用 v1c.2 简化**（`down_node==hit && <10px`）；双击/downTargets 兜底/缩放容忍/Move 超阈值取消 defer v1c.4。
- **Stage tick 管线**：solve→process（多槽，各槽 last_hit）→rematch→render。`cur_hit` 单值字段删（is_pointer_on_ui 改读各槽 last_hit，任一活跃槽命中非根）。Stationary 不刷新 hover（照 fgui 局限，defer v1c.4）。
- **两机约束（v1c.3 执行）**：core+ffi `cargo test --workspace` 180 测全绿；C# 本机写未编译（无 Unity），家里机 EditMode（capture 测骨架补 handle）+ PlayMode（多指/capture demo/鼠标回归）。subagent-driven 6 task 全 Approved + final review Ready（opus）。spec `docs/superpowers/specs/2026-06-23-v1c.3-multi-touch-design.md`、plan `docs/superpowers/plans/2026-06-23-v1c.3-multi-touch.md`、验收 `docs/v1c.3-home-verification.md`。踩坑：PointerKind repr(C) 4B 判别（坑 34）、csbindgen 不生 use-imported struct stub（坑 35）、set_input PointerEvent* 须 fixed-pin（坑 36）。

### 2.18 click 增强（v1c.4，§10.3）— 收尾 v1c
- **click 全对齐 fgui（core input.rs，照 TouchInfo/ClickTest/End/Move）**：`click_test` 取代 v1c.2 `slot_click_ok`——Click 目标 = **down_targets[0]（按下叶，非当前 hit）**，照 fgui「点按缩放」语义（光标漂移到相邻元素仍 click 按下叶）；down_targets[0] 失效（索引越界/重建移除）沿当前 hit 祖先链兜底找首个在 down_targets 内的存活祖先。位移 **per-axis** `|dx|>t || |dy|>t`（非 v1c.2 euclidean）；阈值 **mouse 10 / touch 50**（`click_threshold(touch_id)`，固定像素**无缩放**——「缩放容忍」是伪需求，fgui `_clickTestThreshold` 不乘 scaleFactor）。
- **双击 clickCount 1→2→1**（照 fgui End @1745）：`bump_click_count(slot, button, time_s)` 查 350ms + per-axis 位置 + 同键 → 1→2→1 循环（不到 3），否 1。click_test 返 None（位移超阈值/cancelled）→ reset `last_click_time=0`/`click_count=1`（照 fgui cancel 分支）。EventRecord `pad[0]`→**`click_count:u8`**（offset 5，20B 不变）→ C# `LoomEvent.clickCount` → `EventContext.clickCount`/`isDoubleClick`。**统一核心 350ms**（不依赖 Unity tapCount，跨引擎一致）。
- **Move>50 取消**（照 fgui Move @1705 硬编码 50，mouse+touch 通用）：`is_down && (|dx|>50|||dy|>50)` → `slot.click_cancelled=true` → 下个 Up 的 click_test 返 None。
- **Canceled（偏离 fgui quirk）**：`PointerKind::Canceled=3`（repr(u8)，PointerEvent 16B）→ process `Up|Canceled` 合并臂（Canceled 仅置 `click_cancelled`）= **隐式 CancelClick**。fgui quirk 是 Canceled 仍跑 End clickCount 累加但跳 ClickTest；LoomGUI 改为置 click_cancelled → 不发 Click + reset，更干净（spec §0.6 用户确认）。Up 仍发（onTouchEnd），mouse 路径无 Canceled。C# `LoomInputCollector` 两路径（新 InputSystem / 旧 UnityEngine）`TouchPhase.Canceled`→kind=3。
- **CancelClick API**：`PointerState::cancel_click(touch_id)`（slot 查找同 add_touch_monitor）→ Stage → FFI `loomgui_stage_cancel_click(h, touch_id:i32)` → C# `LoomEventHandler.CancelTouch(int)`。业务用例：拖拽开始取消待 click。
- **stopImmediatePropagation（纯 C#，W3C，fgui 无）**：`EventContext` +`_stopsImmediatePropagation` + `StopImmediatePropagation()`（设两 flag）；`EventBridge.CallBubble/CallCapture` 改 `GetInvocationList()` 逐回调 + immediate break（**null-safe** `ctx != null &&`，保既有 `CallBubble(null)` 测）；BubbleRoute 节点循环已有 `_stopsPropagation` break（immediate 也断冒泡）；capture 节点循环照 fgui 不查 stop。
- **Stationary hover 跟随（fgui 改进）**：v1c.3「Stationary 不刷新」（§2.17 defer 项）→ v1c.4 `process` 头部对**本帧无事件**的活跃槽 re-hit-test `last_pos` + `hover_diff_slot`（静止光标下元素动画移入 → :hover/RollOver/Out 刷新；fgui 依赖 Move 事件无此）。判定纯读 `touch_id`（`used_touch_ids` 集合，**无 find_or_alloc 副作用**）。
- **time_s 复用 tick(dt)**（零新 FFI 参数）：FFI `loomgui_stage_tick(h, dt)`（C# 改传 `Time.unscaledDeltaTime`，照 fgui unscaledTime）→ `Stage::advance_time(dt)`（**tick_and_render 前调**，签名不改）→ `PointerState.time_s += dt`。process 顶部 `let time_s = self.time_s;` 本地化（避 `&mut self` 与 `&mut slot` E0502）。
- **FFI/version**：+`cancel_click`（常驻不 gate）；version v1c.3→v1c.4。EventRecord 20B / PointerEvent 16B / PointerKind repr(u8) 全不变（click_count 复用 pad）。
- **两机约束（v1c.4 执行）**：core+ffi cargo test 全绿（core 162 + ffi 28）；C# 本机写未编译（无 Unity），家里机 EditMode（16 测，BuildStage helper 须填 font_path）+ PlayMode（双击 isDoubleClick / 拖拽>50 取消 / 触摸 Canceled / stationary hover / CancelTouch）。subagent-driven 8 task 全 Approved + final review Ready（opus）。spec `docs/superpowers/specs/2026-06-24-v1c.4-click-design.md`、plan `docs/superpowers/plans/2026-06-24-v1c.4-click.md`、验收 `docs/v1c.4-home-verification.md`。踩坑：borrow_events out_len 是 count 非 bytes（坑 39）、Assert.IsNotNull 对指针装箱 no-op（坑 40）。

### 2.19 drag + longpress + safe-area（v1d.1，§10.3）— 开 v1d
- **drag（opt-in，core input.rs 状态机，照 fgui GObject/UIConfig）**：`Node.draggable`（HTML `draggable="true"` 属性，truthy 仅认 "true"；pkg v4 NodeBlock 末 +1 byte flags bit0 round-trip，formatVersion 3→4）→ Down 时 `drag_target = down_targets.iter().find(draggable && !disabled)`（**leaf 优先 `.iter()` 非 `.rev()`**，最内层 draggable；down_targets=[leaf,…祖先]）+ `drag_testing=true` → Move 超 **drag 阈值 mouse 2 / touch 10**（per-axis OR，`UIConfig.clickDragSensitivity=2`/`touchDragSensitivity=10`，**< click 容忍 10/50**）→ **DragStart**（`EVT_DRAG_START=6` + 置 `click_cancelled=true`——drag 必取消 click）→ 后续 Move **DragMove**(`7`) → Up/Canceled **DragEnd**(`8`)。Canceled 也发 DragEnd（照 fgui onTouchEnd）。**只发事件不跟手**（跟手=自动 SetXY 依赖 transform.translate，留 v1d.3）。drag_target 固定存 slot（无 touch_monitor，Move/Up 恒指向）。disabled 跳过。C# BubbleRoute（为 v1d.5 ScrollPane 祖先接服务）。
- **longpress（universal，core tick 计时，照 fgui LongPressGesture）**：无 flag，任何 down_node 按住 **`LONGPRESS_TRIGGER=1.5s`** 且位移 **≤`LONGPRESS_RADIUS=50px`**（由 Move>50 置 `longpress_cancelled` 间接强制，`LONGPRESS_RADIUS` 与 `MOVE_CANCEL_PX` 同值 50，独立常量明义，`#[allow(dead_code)]`）且未 Up → **LongPress**(`EVT_LONG_PRESS=9`) **一次**（`longpress_fired` guard）。tick 检查放 `process` 头部（stationary-hover 循环后、`events.is_empty()` early-return 前，**空事件 tick 也跑**——按住无 Move 也要触发）。**与 click 独立**（tick 不碰 `click_cancelled`，照 fgui；业务要消费调 CancelTouch）。disabled 跳过。C# BubbleRoute。
- **TouchSlot +6 字段**：`drag_testing`/`dragging`/`drag_target`/`down_time`/`longpress_fired`/`longpress_cancelled`（T3 加全，T4 只加 tick 读——**跨 task 字段共享致 T3 临时 dead_code warnings，T4 消**，预期非缺陷）。
- **EventRecord 仍 20B**：drag/longpress 复用 `event_type:u8` 空位 6/7/8/9（C# `EventType :byte` +DragStart/Move/End/LongPress，纯值域扩展 0-5→0-9，LoomEvent 布局不变）。
- **safe-area（纯 Unity 侧，core 零改）**：`LoomStage._safeArea`（默认 true）→ `ComputeRootTransform` 读 `Screen.safeArea`，`sf=min(area.w/dw, area.h/dh)`（uniform shrink-to-fit），`offX=area.x+(area.w-dw*sf)/2`（**设计 span 居中 safe 区**，非"屏幕中心偏移"），root transform 把 design 映射进 safe 区、safe 区外 letterbox；相机 orthoSize 不变（=sh/2 覆盖全屏）。`LoomInputCollector.ScreenToDesign` 用**同一变换的逐项逆** `dx=(sx-offX)/sf, dy=(offYTop-sy)/sf`（**必须与 ComputeRootTransform 一致，否则触控↔渲染错位**，坑 42）。无刘海屏 safeArea==全屏 → 零回归。**M6 行为变化（spec §5.1 本意）**：设计 aspect≠屏 aspect 时 v1c per-axis stretch → v1d.1 uniform shrink-to-fit+letterbox 居中（v1c render 用 uniform sf 但 input 用 per-axis stretch 本就 latent 不一致，v1d.1 统一）。
- **FFI/version**：version v1c.4→v1d.1；**无新 FFI 函数**（drag/longpress 走既有 borrow_events，csbindgen 不需 regen）。.dll 重编（1694208→1694720B）。
- **两机约束（v1d.1 执行）**：core+ffi cargo test 全绿（core 185 + ffi 30 = 215）；C# 本机写未编译（无 Unity），家里机 EditMode（LoomEventHandlerTests 18 测含 T7 drag/longpress bubble + LoomInputCollectorTests 含 T8 `ScreenToDesign_NotchedSafeArea_RoundTrip` 6 点硬门，BuildStage helper 须填 font_path）+ PlayMode（drag opt-in/取消 click/阈值 + longpress 1.5s 一次/Move>50 取消/独立 click/disabled + safe-area Device Simulator 避刘海/触控↔渲染对齐/关 _safeArea 回归）。subagent-driven 8 task 全 Approved（T8 一轮 Critical fix）+ final review Ready（opus，8 跨 task 集成点全清）。spec `docs/superpowers/specs/2026-06-24-v1d.1-drag-longpress-safearea-design.md`、plan `docs/superpowers/plans/2026-06-24-v1d.1-drag-longpress-safearea.md`、验收 `docs/v1d.1-home-verification.md`。踩坑：跨 crate 签名变更漏改（坑 41）、safe-area forward/inverse 变换不一致（坑 42）。

### 2.20 键盘 + 焦点 + Tab + `:focus`（v1d.2，§10.3）
- **焦点状态（单一全局，照 fgui Stage.focus）**：`Scene.focused_node: Option<NodeId>` + `Node.focused: bool`（:focus 伪类源）+ `Node.tabindex: Option<i32>`（HTML `tabindex` 属性，`v.parse::<i32>().ok()` 非数字→None；None=不可聚焦/`Some(-1)`=仅编程/`Some(0)`=DOM序/`Some(N>0)`=显式序）。pkg v4→**v5** NodeBlock flags 后 +`tabindex:i32`（None→`i32::MIN` 哨兵 round-trip），formatVersion MIN=MAX=5（旧 v4 拒）。`Scene::build` 6→7-tuple。
- **focus 通道（单源 `pub(crate) fn focus_node(scene,new,out)`，input.rs 模块级）**：设/清 `focused_node`+`node.focused` + 发 `FocusOut`@旧/`FocusIn`@新（同目标 no-op）。**3 处共用**：① Stage tick 最前消费 `pending_focus_request`（request_focus/blur 记，**不直写 last_events**——避免 tick 覆盖丢事件）；② process Down arm **click-to-focus**（沿 down_targets 找最近 `tabindex>=0` 非 disabled，`.copied()` 释放 slot 借用再 `focus_node(&mut scene)`）；③ `process_keys` Tab 导航。`request_focus` **强制**聚焦任意非 disabled 节点（含 tabindex=None/-1，照 fgui RequestFocus 不查 focusable；disabled 拒）。
- **Tab 链 + 导航（core `build_tab_chain`/`next_focus`/`process_keys`）**：链 = 正整数 tabindex 升序（stable 同值保 DFS）后接 tabindex=0 组（DFS 序），-1/None/disabled 排除。Tab/Shift+Tab(`KEY_TAB=9`+`MOD_SHIFT`) 按 `next_focus` 移焦（链内 ±1 wrap，链外→首/尾）。**Tab 被导航消费不发 keydown**（照 DOM Tab 默认动作=移焦；业务拦 Tab 留 v1.x preventDefault）。空链 Tab no-op。
- **keydown/up（core process_keys）**：新 `KeyEvent{key_code:u32,modifiers:u8,is_down:bool,pad:[u8;2]}` 8B 输入（C# `set_key_input`）→ 有焦点才发 keydown/up（无焦点丢弃），**复用 EventRecord 流**（`EVT_KEY_DOWN=12`/`UP=13`/`FOCUS_IN=14`/`FOCUS_OUT=15`，`node_id`=焦点节点、`touch_id`复用装 key_code、`pad[0]`=modifiers、x/y=0，**零新输出 ABI**）。modifiers 位掩码 bit0=shift/1=ctrl/2=alt（照 fgui InputEvent，砍 cmd）。不范围：IME/character、TextInput、`:focus-visible`（随 v1.x）。
- **tick 管线（§15，v1d.2 增 ⓪+②）**：⓪消费 pending_focus_request(`focus_node`) → ①solve → process(含 click-to-focus) → ②`process_keys`(keydown/up+Tab) → last_events= → rematch → render。`:focus` 靠 `Compound.pseudo_focus`（dynamic.rs 单一定义，selector.rs `pub use` 重导出）+ `compound_matches_with_state` 门控 `node.focused` + `extract_dynamic_rules` 纳入 `:focus` 规则，rematch 每帧跑自动吃焦点变化。
- **FFI/version**：version v1d.1→v1d.2；**3 新常驻 FFI**（`set_key_input`/`request_focus`/`focused_node`，csbindgen reimport regen）+ `KeyEvent` struct。.dll 重编（1694720→1709056B）。C# `EventType`+4(12-15)/`LoomEvent.modifiers`@6/`EventContext.keyCode`(uint touch_id)+`modifiers`/`DispatchPending`+4 BubbleRoute/`LoomInputCollector.CollectKeys`(新旧输入系统+KeyList 白名单+CurrentModifiers)/`LoomStage` LateUpdate 调 CollectKeys。
- **两机约束（v1d.2 执行）**：core+ffi cargo test 全绿（core 206 + ffi 35 = 241）；C# 本机写未编译（无 Unity），家里机 EditMode（LoomEventHandlerTests 20 测含 T7 KeyDown/FocusIn bubble，BuildStage helper 须填 font_path）+ PlayMode（tabindex opt-in/click-to-focus/Tab 导航 wrap/keydown 需焦点+Tab 消费不发/:focus 伪类/request_focus 下 tick 生效）。subagent-driven 7 task 全 Approved + final review Ready（opus，C1 Critical 修：C# KeyEvent struct 漏手补，坑 35 复发）。spec `docs/superpowers/specs/2026-06-24-v1d.2-keyboard-focus-design.md`、plan `docs/superpowers/plans/2026-06-24-v1d.2-keyboard-focus.md`、验收 `docs/v1d.2-home-verification.md`。踩坑：csbindgen struct stub 复发（坑 35 强化）、Scene 加字段 plan 漏枚举构造点（坑 43）。

### 2.21 transform 命中渲染 + NativeHost-lite（v1d.3，§10.3）
- **transform = core 算累计世界矩阵，后端扁平**（spec §3）：`Affine2=[a,b,c,d,tx,ty]`（列主序 `x'=a·x+c·y+tx`，新 `loomgui_core/src/transform.rs`）+ free fn（`IDENTITY`/`from_translate/from_rotate/from_scale/mul(&m,&n)=m∘n`/`inverse`/`apply_point`/`is_pure_translation`）+ `Affine2Ext` trait（链式 by-value，Copy 类型按值地道）。`compute_world_transforms` 每 frame DFS：`local = T(rel)∘T(pivot)∘transform.matrix∘T(-pivot)`（**pivot=box center 固定**，rel=layout_rect.xy−父.xy，root rel=自身 xy）；`world=parent.world∘local`。借用安全：DFS 算局部 `worlds:Vec` 纯读 nodes，末尾 `scene.world_transforms=worlds`（避 &mut/& 冲突）。
- **LocalTransform 存 Affine2 非 TRS 分解**（关键，spec §2.1 修订）：单节点 `scale(2,1) rotate(45deg)` 复合 = 剪切矩阵，存分解字段（tx/rot/scale）会在提取时**丢剪切**。故 `LocalTransform{matrix:Affine2}` 解析期 `mul` 累积。CSS 解析（mapping.rs `parse_transform`）：`translate(px,px)/rotate(deg)/scale(num[,num])` 左乘累积（最左最外层）；`skew/matrix()/%` 静默跳过；`iter_transform_funcs` 拆函数（无嵌套括号支持）。
- **渲染两路径**（spec §3.5/§3.6，关键正确性）：**identity/merge 节点**走现有 TRS（顶点绝对 layout_rect + blob re-base 减 tx,ty→top-local + GO localPosition=tx,ty + 现有 material，**零回归**）；**非纯平移节点** break merge + 走 matrix shader（顶点 box 本地 (0,0,w,h) + blob 不 re-base + GO transform=identity + `_ObjectMatrix` uniform + shader `OBJECT_MATRIX` variant 顶点 `mul(_ObjectMatrix,float4(pos.xy,0,1))`→world）。两路径判断都用 `is_pure_translation(wm)`（a≈1&&b≈0&&c≈0&&d≈1，epsilon 1e-6，Rust↔C# 对齐）。剪切（任意仿射含非均匀缩放∘旋转）matrix 天然支持。
- **命中 world_to_local**（spec §3.4）：`local_point = inverse(world).apply_point(point)` → 判本地 box `(0,0,w,h)`（top-left 原点，不用 layout_rect.x/y）。仿射逆含剪切可逆（det≠0）。clip 门控不变（世界 AABB，clip **不随 transform 旋转**，spec 决策 5 保守留 v1.x）。
- **blob v3→v4 + pkg v5→v6**：frame blob transform 列 `local_x/local_y`(2) → world matrix `m_a,m_b,m_c,m_d,m_tx,m_ty`(6)（列序 node_id@0,parent_id@1,visible@2,alpha@3,sort_key@4,mask_context@5,m_a@6..m_ty@11,payload_kind@12,mesh_off@13,mesh_len@14,text_off@15,text_len@16,tex_id@17，**18 列**，header_len=12+18*4=84，arena Mesh@84/Text@92/Clip@100）。pkg `ResolvedStyle`+transform → bincode 变 → formatVersion 5→6。**两套 version 独立**（pkg v6 Rust-internal / blob v4 FFI 跨语言，C# IsValid 校验拒 v3）。version 串 v1d.2→v1d.3。
- **NativeHost-lite（后端纯 C#，core 零改）**（spec §3.7）：HTML 普通 `<div id>` 占位（core 不认新 kind，当 Container 算 world matrix）；`NativeHostManager`（Runtime/）`Dictionary<uint,GameObject>` + `Bind(uint|string via FindNodeById)/Unbind/Clear/Sync(blob)`。Sync 每帧 node_id→i 扫，从 world matrix **TRS 分解**（rot=atan2(b,a),sx=√(a²+b²),sy=√(c²+d²),pos=(tx,ty,0)；剪切降级）设外部 GO + sort_key→Renderer.sortingOrder + node 消失 SetActive(false)。**复用既有 `find_node_by_id` FFI**（不新增）；零新渲染 FFI。**v1-showcase 验收修正（坑 72）**：GO 挂 root handedness flip（det<0 → Cull Back 剔除）→ 建 `_container`(挂 root, localScale=(1,-1,1)) 翻正 worldScale=(sf,sf,sf) + per-node wrapper（fgui GoWrapper cachedTransform 两层结构）保留用户 GO scale；GO layer=LoomUILayer + material renderQueue=3000 + sortingOrder=sort_key（照 fgui GoWrapper 渲染顺序 3 机制）。
- **调试 dump**（spec §3.8）：新 `loomgui_stage_dump_scene(h,*len)->*const u8` FFI（StageHandle 加 `dump_blob:CString`，下 tick 失效）+ core `dump_scene_json`（整树 JSON：node_id/parent/tag/id/classes/kind/layout/world_matrix/visible，id/classes **JSON 转义**）。C# `LoomStage.DumpScene()`（null-stage guard）。
- **Unity 后端**：shader 加 `multi_compile _ OBJECT_MATRIX`（×CLIPPED 4 variant）+ `_ObjectMatrix` Matrix property/CBUFFER；`MaterialManager.Key` +`matrixFlag:bool`；`FrameBlob` v4 列读取（Ma..Mty/IsPureTranslation）；`MirrorPool.Sync` 双路径。**非纯平移 _ObjectMatrix 用 MaterialPropertyBlock**（不污染共享 material，坑 47）+ bounds 平移到世界（坑 48）。**v1-showcase 验收修正（坑 73 三层）**：① `_ObjectMatrix` 拆 4 Vector Properties（MPB SetVector ×4，73① 纠坑 52——非 Properties CBUFFER 字段 MPB 也不覆盖）；② `GetRow`（配 HLSL `float4x4(v0..v3)` row-major，73②）；③ 删 I1 fix bounds translate（73③ 纠坑 48——mutate mesh 资产致 pure 切回双 translate culling 误剔），非 pure 也 GO localPosition=(Mtx,Mty)（translate 进 GO，_ObjectMatrix 只 scale/rotate）→ renderer.bounds 自动 world。
- **FFI/version**：version v1d.2→v1d.3；1 新常驻 FFI（`dump_scene`，csbindgen regen）+ StageHandle+dump_blob。.dll 重编（1709056→1733120B）。
- **两机约束（v1d.3 执行）**：core+ffi cargo test 全绿（core 230 + ffi 37 + snapshot 3 = 270）；C# 本机写未编译（无 Unity），家里机 PlayMode（旋转/剪切/缩放容器视觉+子跟随/剪切走 matrix shader/命中 world_to_local/identity 不回归/NativeHost cube 跟随+显隐/DumpScene 日志/500 节点 stress）。subagent-driven 11 task 全 Approved + final review Ready（opus，跨语言矩阵契约全链核实正确，无 Critical；I1 culling+M1 MPB fix）。spec `docs/superpowers/specs/2026-06-25-v1d.3-transform-nativehost-design.md`、plan `docs/superpowers/plans/2026-06-25-v1d.3-transform-nativehost.md`。踩坑：Affine2 type/mod 同名 namespace 冲突（坑 46）、matrix shader GO identity 致 Mesh.bounds 剔除错位（坑 48）、共享 material _ObjectMatrix 覆盖（坑 47）。

### 2.22 GTween tween 引擎（v1d.4，§10.3）
- **TweenManager + replace-override**（spec §3-§5）：新 `loomgui_core/src/tween.rs`——`TweenProp`/`Ease` 均 `#[repr(u8)]`（Opacity=0..TextColor=5 / Linear=0..BackInOut=9）+ `Ease::evaluate(t,dur)->f32`（10 个照 fgui EaseManager 直译，OVERSHOOT=1.70158；dur<=0 返 1）+ `prop_value_size`（1/2/4）+ `try_from(u32)` 边界校验。`TweenManager.update(dt, scene, &mut out)` 每 tick：`elapsed+=dt`→delay 门控（`<delay` 跳过不写）→`tt=elapsed-delay` 钳到 duration→`norm=evaluate`→`apply` 写通道→`tt>=duration` 产 `EVT_TWEEN_COMPLETE`+killed→`retain(!killed)`。Stage 持 `tweens`。
- **anim override = Scene transient 字段**（spec §4，关键）：`Scene.anim: AnimTable`（`Vec<NodeAnim>`，同 `world_transforms` **不进 pkg**）。`NodeAnim{opacity/transform/bg_color/text_color}` 全 Option。**replace-override**：4 读取点 `unwrap_or(CSS)`——`compute_world_transforms` 读 `anim.transform.unwrap_or(style.transform.matrix)`（**不 compose，覆盖 css_matrix**）；`build_render_nodes` 读 `anim.opacity/bg_color/text_color.unwrap_or(style.*)`。`AnimTable::get` 经 `NodeAnim::is_empty` 过滤（全 None→None）→ 热路径退回 CSS **零回归**。一节点一 transform tween（Translate/Scale/Rotation 共享 transform 通道，并发 last-write-wins；混用嵌套 div）。持久：killed/自然完成停末值，`clear_anim(_prop)` 才回 CSS。**颜色通道（bg_color/text_color）用 0-1 归一化**（mapping.rs:77 hex `/255.0`、render 测试 `[0,0,1,1]`=蓝；brief/草稿常写 0-255 致 clamp 全白——v1-showcase T8 implementer 查源修 `Rgba(/255f)`）。
- **时钟 = 单 unscaled dt stash**（spec §6）：`advance_time(dt)`（FFI tick 已先调）加 `self.pending_dt=dt`；`tick_and_render`（**无参签名不动**，零测试 ripple）顶部 `let dt=pending_dt.take(); tweens.update(dt,scene,&mut out)`——**须在 solve/compute_world_transforms 前**（anim 先写后读）。load_inline/load_package 调 `tweens.clear()`（防悬空 node_id）。per-tween timeScale/scaled dt defer v1.x。
- **EVT_TWEEN_COMPLETE=16 复用 EventRecord 字段**（spec §8，零结构改动）：`click_count`=prop(u8)、`touch_id`=tag(i32)、x/y=0。C# `EventType.TweenComplete=16` + `DispatchPending` 加 `case→DirectDispatch`（target-specific 不 bubble）；listener 读 `ctx.clickCount`=prop、`ctx.touchId`=tag。
- **FFI/version**：version v1d.3→v1d.4；4 新常驻 FFI（`tween/kill_tween/clear_anim/clear_anim_prop`，csbindgen regen）+ `TweenProp/Ease::try_from(u32)`。.dll 重编（1733120→1739776B）。**blob 保持 v4 不 bump**（anim 在 core fold 进既有字段，blob/MirrorPool 零改）。
- **两机约束（v1d.4 执行）**：core+ffi cargo test 全绿（core 249 + ffi 38 + snapshot 3 = 290）；C# 本机未编译，家里机 PlayMode（fade-in / pop-in BackOut overshoot / 并发不同通道 / onComplete tag+prop / 颜色 / kill 停末值 / clear 回 CSS / 零回归 / 500 节点 stress）。subagent-driven 8 task 全 Approved + final review Ready（opus，跨语言 u32↔repr(u8) 三处对齐 + EventRecord 20B ABI 不变 + 零回归每读取点核实，无 Critical/Important）。spec `docs/superpowers/specs/2026-06-25-v1d.4-gtween-tween-design.md`、plan `docs/superpowers/plans/2026-06-25-v1d.4-gtween-tween.md`。踩坑：LoomGUIBindings.cs 是 csbindgen 自动生成+gitignored（坑 50）、brief 再次写错 borrow_events len（坑 39 复踩）、Scene 加字段 replace_all 技法（坑 43 优化）。

### 2.23 ScrollPane + 滚动条 + 滚轮 + 手势仲裁（v1d.5，§10.3/§12.7）— v1d 收尾，关验收 #2
- **overflow 扩展（CSS 标准，pkg v7）**：`ResolvedStyle.overflow_hidden:bool` → **`overflow_x/overflow_y: OverflowMode`**（`#[repr(u8)]` Visible=0/Hidden=1/Scroll=2/Auto=3，Default Visible 零回归）。CSS `overflow` shorthand 设双轴、`overflow-x/y` longhand 设单轴；未知值宽松忽略。任一轴≠Visible → `clip_rect=Some(border 框)`（复用现有 clip 基建）。bincode 变 → pkg formatVersion 6→7。**scroll 能力 derive**：`capable=overflow∈{Scroll,Auto}`；`effective=capable && (Scroll || content>viewport)`（auto 无溢出→不候选不显条；scroll→始终候选）。
- **scroll.rs 数据模型（transient 不进 pkg，同 anim/world_transforms）**：`ScrollPaneState{content_size/viewport_size/overlap/scroll_pos/velocity/tweening(0无/1编程/2惯性回弹)/tween_start/change/time/duration + content_size_dirty}`（全 `(f32,f32)` 元组，core 无 Vec2）。`ScrollTable(Vec<Option<ScrollPaneState>>)` NodeId 索引（get/get_mut/ensure/clear，镜像 AnimTable 但 Option 槽——scroll 容器是少数）。`refresh_content_sizes(scene)` solve 后填：content_size=**直接子 layout_rect AABB**（照 fgui GComponent.UpdateBounds，不包 padding/margin），viewport=content box（v1 简化用 border box，建议 scroll 容器 padding:0），overlap=max(0,content-viewport)。
- **offset = 虚拟平移注入 `compute_world_transforms` DFS（核心，零 DOM 改）**：`rec` 里 `world[N] = world[父] ∘ T(-父.scroll_pos) ∘ local[N]`（父是滚动容器时，查 `scene.scroll.get(node.parent)`）。**容器自身 world 不含自己 scroll_pos**（其 world 用它父 offset）；后代每层累积。符号：scroll_pos.y 增大（向下滚）→ 内容 y 负移（上推，照 fgui `container.SetXY(-xPos,-yPos)`）。**复用 v1d.3 world matrix + 现有 clip**（viewport clip_rect 固定，shader `_ClipBox` 裁滚出部分；命中 world_to_local 天然含 offset）→ **blob v4/MirrorPool/shader 零改**（scroll_pos 折进既有 m_a..m_ty）。scroll_pos=0 → T(0,0)=identity → no-op 零回归。
- **物理自维护 tween（不走 GTween，§12.7）**：`ScrollPaneState::{drag_follow/begin_inertia/begin_bounce/advance/apply_wheel/set_pos}`。常量照搬 fgui：DECELERATION_RATE=0.967（**f64**，log 精度）/TWEEN_TIME_DEFAULT=0.3/PULL_RATIO=0.5（越界打折+cap viewport*0.5）/BOUNCE_THRESHOLD=20/SCROLL_STEP=25/INERTIA_THRESH_PC=500/TOUCH=1000/INERTIA_DIST_COEFF=0.4。**cubic_out=(t-1)³+1** 所有 tween 共用。drag 跟手 1:1 + 越界 PULL_RATIO 打折 + 速度 exp 平滑（Lerp(vel,Δ/dt,dt*10)）；Up 惯性 `duration=log(60/|v|,0.967)/60`（**|v| 非 v²**，坑 54）min 0.3，距离 `v·dur·0.4`；回弹**非弹簧**——超 20px cubicOut 0.3s 回边界。**禁 GTween 直接 tween scroll_pos**（API 层无入口，避免双写）。
- **手势仲裁（per-slot，fgui 全局静态→LoomGUI 每槽独立）**：`TouchSlot` +scroll_candidate/scroll_testing/scrolling_pane/scroll_gesture/grip_dragging 字段。Down 沿 down_targets 找最近 effective 容器作候选；Move **阈值赛跑**（scroll mouse8/touch20 > drag mouse2/touch10，但 drag 阈值更小常先达）——**先达者赢**：scroll 赢→scrolling_pane 设+click_cancelled+drag_target=None；drag 赢→scroll_testing=false。**轴锁**：V-only（effective_y&&!effective_x）遇 dx>dy 让出→`next_effective_ancestor` 提升候选或清；Both 两轴都跟。**嵌套**：最内层候选优先，让出沿链提升。Up→begin_inertia(is_touch)。
- **滚轮（新输入通道，不产事件）**：`WheelEvent{#[repr(C)] x,y,delta_x,delta_y f32}16B` + FFI `set_wheel_input`（累积式 pending_wheel）+ `apply_wheel_to_hit(scene,w)`（hit→沿祖先找最近 effective→apply_wheel）。**不加 PointerKind::Wheel**（PointerEvent 16B 装不下 delta）——独立输入 struct。无 EVT_SCROLL（无消费者，defer v1.x）。新旧输入双路径（滚轮用旧 Input.mouseScrollDelta 够，坑 45）。
- **滚动条（合成节点 + 专用命中，新路径）**：`build_render_nodes` 末尾对 effective 容器追加合成 thumb RenderNode——`node_id=container|V_THUMB_FLAG(0x4000_0000)`/`H_THUMB_FLAG(0x2000_0000)`（**sentinel 高位**，真实 NodeId 小不撞；MirrorPool 当普通 quad），sort_key=内容 max+1（assign_sort_keys **后**取，坑 55），mask_context=0（不裁剪），world_matrix=IDENTITY，半透明灰。`hit_scrollbar_grip(scene,point)` 算 thumb design-rect，`hit_test` **前置**返 sentinel thumb_id。grip Down（scroll 候选前）→grip_dragging+CaptureTouch；Move perc→scroll_pos（非 delta）；Up 清不惯性。
- **tick 重排（§8，偏离 §12.7(2b')）**：`tween→focus→solve→refresh_content_sizes→process(仲裁+跟手)→wheel 消费+advance_all(惯性/回弹)→keys→compute_world_transforms→events→rematch→render`。**compute_world_transforms 从 solve 后移到 process 后**——读 scroll_pos（drag+inertia+wheel）同帧进 world matrix，**零拖拽延迟**。process 的 hit_test 用**上帧** world_transforms（1 帧差，§8.2 认：仲裁在 Down 未滚动前判定，clip viewport 固定主导）。首帧 guard（world_transforms 空→solve 后算一次）防 hit OOB。
- **FFI/version**：version v1d.4→v1d.5；2 新常驻 FFI（`set_wheel_input`/`set_scroll_pos`，csbindgen regen）；**手补 C# 镜像 LoomGUIWheelEvent.cs**（坑 35）。**pkg v7 / blob 保持 v4**（scroll 折进 world matrix）。.dll 重编（1739776→1755136B）。
- **两机约束（v1d.5 执行）**：core+ffi cargo test 全绿（core 318+ffi 40+snapshot 3=361≈356 报告口径）；C# 本机未编译，家里机 PlayMode（拖拽跟手/Up 惯性/边界回弹/滚轮/grip/auto 不显条·scroll 始终显条/嵌套轴锁/scroll-vs-draggable/SetScrollPos+零回归+500 stress）。subagent-driven 12 task 全 Approved + final review Ready（opus，跨任务 seam offset↔仲裁↔tick↔sentinel↔render/hit 逐链 sound，无 Critical/Important）。spec `docs/superpowers/specs/2026-06-26-v1d.5-scrollpane-design.md`、plan `docs/superpowers/plans/2026-06-26-v1d.5-scrollpane.md`。踩坑：fgui `v2` 变量名误导 v²→|v|（坑 54）、合成 sentinel id 跨链致 apply_wheel OOB（坑 55）。

### 2.24 v1e dirty hash + Unchanged emit（§614-623 blob 契约兑现）— 关 v1-scope §4 性能基线
- **机制**：Stage 持 `prev_node_hashes: Vec<u64>`（transient，**Stage 字段非 Scene** → 天然不进 pkg）。`build_render_nodes` 签名 `(scene,font,tex, prev_hashes:&[u64]) -> (FrameData, Vec<u64>)`：逐节点 emit payload 后调 `render::dirty::node_hash(&rn)`，与上帧等（且 `prev_hashes.len()==n_nodes` 守基线）→ payload 改回 `NodePayload::Unchanged`。`load_inline/load_package` clear 基线（防 reload NodeId 错位）；首帧/空基线全 emit（零回归）。
- **node_hash 字段集**（DefaultHasher，f32 走 to_le_bytes）：world_matrix 6 列（**含 scroll_pos**——scroll 子树 world 变→hash 不等→自动重传，不需特殊处理）+ visible/alpha/grayed/color_tint/blend + payload 摘要（Mesh: texture+verts.len+colors[0]+**verts[0]/verts[2] 首末顶点**；Text: font_size+color+glyph_count+首字 codepoint+**首字 pen_x/pen_y**）。**不含 sort_key/mask_context**——它们在 `assign_sort_keys` 之前调 node_hash 时是占位值（0），hash 无贡献；结构变必伴随节点增删（baselined=false 全 dirty）或 world/payload 变（hash 仍捕获），故不 hash。**全链路 v0 预留兑现**：`NodePayload::Unchanged`（node.rs）+ blob kind=0（blob.rs:155）+ C# 读 kind（FrameBlob.cs:36）+ C# `kind!=1&&!=2 continue`（MirrorPool.cs:71）从 v0 全通，v1e 只在 Stage 加 hash 跟踪 → **FFI/blob(v4)/pkg(v7)/C# MirrorPool 四零改**。
- **合成 scrollbar 强制 emit**（不进 hash 比较，随 scroll_pos 变、数量少）；merge 对 Unchanged passthrough（merge.rs:33）。**ponytail 天花板**：hash 碰撞最坏 1 帧视觉延迟不破正确性（标 `// ponytail:`）；**真风险是 hash 字段遗漏**（某视觉字段没进→变了不重传→持续错），reviewer 须逐项对照 blob 公共头列——见坑 56（final review 抓的 3 处遗漏）。
- **验收（双轨）**：criterion bench 500 节点——静态帧 476µs（全 Unchanged，比冷帧快 2.7× 证 dirty 生效）/ 冷帧 1.28ms / 换页帧 1.19ms，均 ≤2ms（v1-scope §4 过线）。C# `_frameBuf` 改 ArrayPool（冷帧零 GC，ReadMesh per-node alloc 留观察撞墙再上）。家里机待验：PlayMode Profiler 静态帧≈0 upload + 冷/换页帧≤2ms + GC Alloc 静态帧≈0。

### 2.25 border-radius 圆角 mesh（v1.2，§8.2 MeshFactory）
- **机制**：`render/mesh.rs::rounded_rect(rect,color,radii:&[(f32,f32);4],uv_min,uv_max)` 产三角扇（中心点 idx0 + 4 角弧顶点，三角形 `(0,i,i+1)` 末尾回 1）。radii 序 [TL,TR,BR,BL]，每元 (h,v) 像素（Container/Button 分支渲染期 `%` resolve：h=width×pct, v=height×pct，ResolvedStyle 存 CSS 原始 `LengthPercentage`）。照搬 fgui `RoundedRectMesh`：自适应分段 `ceil(π·max(rx,ry)/8)+1`（最小 2）、末段精度锁 `start+π/2`、直角分支（rx<=0||ry<=0 → 单顶点）。两改进：①CSS 按边缩放钳制 `scale=min(1,w/(tl_h+tr_h),w/(bl_h+br_h),h/(tl_v+bl_v),h/(tr_v+br_v))`（vs fgui per-corner min，不对称四角不过度钳）②角序 TL→TR→BR→BL。
- **零回归分流**：Container/Button 分支 `all_zero = 四角全 (rx<=0||ry<=0)` → 走旧 `quad`（4 顶点，dirty hash 不变）；否则 `rounded_rect`。v-flip 调用点交换 uv v（同 quad/Image）。与 background-image 共存：UV 线性映射到 `fit_uv` 子区（cover/contain/stretch 自然成立）。
- **直角分支落矩形顶点**（坑 77）：corners 元组附矩形顶点 corner，rx<=0||ry<=0 时直接 push corner，**不靠圆心+方向算**（rx=0 ry>0 时圆心+sin·ry 偏离角顶点 → 镂空）。设计 §5.1 注预见的"硬编码角顶点"方案。
- **dirty hash**：圆角 mesh >4 顶点时 hash **全顶点/全 UV**（坑 76）；quad 仍采样首末。PKG_FORMAT_VERSION 8→9（坑 74，加 `border_radius` 字段）。

## 3. 依赖 API 适配踩坑（v0 最大教训）

> **plan/brief 写的 API 草稿常与实际 crate 版本不符**。遇编译错按本节对照，**勿硬改依赖版本**，按 crate 实际源码（`~/.cargo/registry/src/<crate>-<ver>/src/`）调。

### 3.1 taffy 0.5（layout/mod.rs）
- **无 `MeasureFunc::Boxed`**。用 `TaffyTree<NodeContext>` + `new_leaf_with_context(style, ctx)` + `compute_layout_with_measure(root, Size::MAX_CONTENT, FnMut)`。
- measure 闭包签名：`FnMut(Size<Option<f32>>, Size<AvailableSpace>, NodeId, Option<&mut NodeContext>, &Style) -> Size<f32>`。`known.width` 是 `Option<f32>`（Some=约束宽，None=不限）。
- **闭包可借 `&font`**（FnMat 调用期存活，非 `'static`）→ **不需要 `Arc<Font>`**（v0 一度误判要 Arc，实际单 FnMut 借用合法）。
- `Size::MAX` → `Size::MAX_CONTENT`。
- 根 size setter 用 `Dimension::Length`（`Style.size` 是 `Size<Dimension>`）。
- `Style` **无 `order` 字段**（CSS order 无法存 taffy；留 `ResolvedStyle.order` 待 v1 消费）。
- **`Style.overflow: Point<Overflow>`**（taffy 0.5，Overflow=Visible/Clip/Hidden/Scroll）——CSS flex §4.5 automatic min-size：overflow≠Visible 的 flex item min-size=0（不被 content 撑开）。**必须显式设**（LoomGUI OverflowMode→taffy Overflow 同步），否则默认 Visible→min-size=min-content→scroll 容器被 content 撑开 overlap=0（坑 59）。构造 `taffy::geometry::Point { x, y }`。

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
- **ShaderLab Properties 类型**：只 Float/Range/Int/Color/Vector/2D/3D/Cube，**无 Matrix**（坑 52）。per-renderer matrix uniform 放 `UnityPerMaterial` CBUFFER（无 Properties 对应）+ MPB.SetMatrix 覆盖。
- **MaterialPropertyBlock 覆盖范围**：只覆盖 material property（Properties 或 `UnityPerMaterial` CBUFFER 字段）；CBUFFER 外全局 uniform MPB **不覆盖**（坑 52，静默失效）。per-renderer uniform 必须进 CBUFFER。
- **TransformObjectToWorld = mul(unity_ObjectToWorld, pos)**：GO 是 root 子时 = root_ObjectToWorld（含 root transform sf/-sf/sf+rootPos）。core 算 design world，shader 桥接 design→Unity world 用它（坑 51）。GO transform=identity 时仍 = root transform（继承父）。
- **Time.unscaledDeltaTime 首帧 spike**：PlayMode 首帧可达数秒（加载延迟，实测 2.07s），tween/动画别在 Start 自动播（坑 53，瞬间 complete 写末值）。

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

### 坑 1：taffy 0.5 `MeasureFunc::Boxed` 不存在（API 详见 §3.1）
brief 写 `MeasureFunc::Boxed` 编译失败 → 0.5.2 改 `TaffyTree<NodeContext>` + `compute_layout_with_measure` FnMut（`Arc<Font>` carry 作废，FnMut 借用合法）。**教训**：brief API 草稿是起点非权威，按编译器 + crate 实际版本调。

### 坑 2：ttf-parser 0.20 advance/kerning API 改名（API 详见 §3.2）
`glyph_advance_width`/`kerning_for` 编译失败 → 0.20 改 `glyph_hor_advance`(返 u16) + kerning 移 `kern::Subtable`。**教训**：ttf-parser 跨版本 API 变动大，查 `~/.cargo/registry` 源码确认。

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
v2→v3 加 tex_id 列，某 C# builder 只升 header（14 列）没补 data 列写 → 4 字节 skew，`ReadMesh` 读串。手搓 blob fixture 散落多文件多 builder，version bump 后 review 只抽查漏一个。**解决**：grep 全 C# 测目录 `version=Nu`/`HeaderLen`/`elemSize = {`/`N \* 4` 逐 builder 升（version/HeaderLen/offs/elemSize 项数/loop 边界/补列写）；Rust `num_col_offsets=columns.len()` 自动传播，C# arena offset 基准 `12+N*4` 也改。**教训**：blob 是 Rust↔C# 字节契约，version bump = 全仓 fixture 同步事件，grep 枚举所有 builder 不能只改抽查的。

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
**教训**：打包器产两文件（.pkg.bin + atlas.png）时，**磁盘 atlas 名必须 == header 的 `atlas_filename`**（后端按 header 载）；用同一变量拼两端，别各算各的。**v1-showcase 加 `-a <name>.atlas.png`/`pack_named`**（c65db2f）——多 sample 共存 StreamingAssets 用独立 atlas 名，避免共享 `loom.atlas.png` 互相覆盖（LoomStage 按 pkg header `atlas_filename` 载，非 hardcode）。

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

### 坑 32：implementer 为让 brief 测通过改实现（已恢复，v1c.2）
implementer 为坑 31 brief 测 `out.is_empty()` 通过，改 Move handler 抑制 EVT_MOVE——超 scope + 破坏 §7.1 恒产 + 影响drag。fix 恢复 Move 无条件 emit + 改测断言（坑 31）。**教训**：brief 测 vs 既有语义冲突**改测不改实现**；implementer 遇冲突应 flag DONE_WITH_CONCERNS 让 controller adjudicate，controller review 验「超 brief scope 改动」。

### 坑 33：C# EventBridge internal 测跨 namespace 不可见（v1c.2）

**症状**：T3 `EventBridge`（internal）单测在 `LoomGUI.Tests` namespace 直接 `new EventBridge()`——跨 namespace 不可见，编译报 inaccessible。
**根因**：v1c.1 测只碰 public 类型，无此需求；v1c.2 EventBridge internal + 测直接构造触发。
**解决**：`EventBridge` internal → public（测访问 + 避 `InternalsVisibleTo` 配置；EventBridge public 无害，业务通过 AddListener 用不直接 new）。
**教训**：C# 类型可见性要考虑测访问——单测直接构造的类型须 public 或加 `[InternalsVisibleTo("Tests")]`（项目已有此机制，坑 13 同源 csbindgen InternalsVisibleTo）。

### 坑 34：`#[repr(C)]` enum 无显式整型 → 判别默认 isize=4B 非 1B（v1c.3）

**症状**：v1c.3 PointerEvent 设计预期 16B（kind 1B + button 1B + pad 2B + touch_id 4B + x/y 8B），但 `cargo` 实测 20B——C# 若声明 PointerKind:byte 会 mis-slice 整个输入数组。
**根因**：`PointerKind` 是 `#[repr(C)] pub enum { Down=0, Up=1, Move=2 }` 无 `repr(uN)` → Rust 用平台 isize（4B）作判别，非 1B。spec/plan 草稿想当然以为 C-like enum 1B。
**解决**：`PointerKind` 加 `#[repr(u8)]` → 1B 判别，PointerEvent 回 16B。C# PointerEvent.kind 对齐 byte。
**教训**：FFI 边界的 C-like enum **必须显式 `#[repr(uN)]`**（u8/u16/u32），否则判别 isize 跨平台不稳 + 撑大 struct。永远 `size_of::<T>()` 断言 ABI struct 尺寸，别信草稿。本坑由 T3 implementer 诚实上报（初版把 sizeof 断言改成实际 20，controller 决定修 core repr(u8) 回 16 而非接受 20）。

### 坑 35：csbindgen 不为 use-imported 的 `#[repr(C)]` struct 生成 C# stub（v1c.3，v1d.2 复发）
csbindgen 只扫 `#[no_mangle] fn` 签名不追 `use` 路径 → FFI struct（PointerEvent/EventRecord/KeyEvent）无 C# stub，编译报找不到类型。**解决**：手补 C# 镜像（`[StructLayout(Sequential)]` 字段序对齐 Rust `#[repr(C)]`）——PointerEvent→`LoomGUIPointerEvent.cs`、EventRecord→手写 `LoomEvent`、KeyEvent→`LoomGUIKeyEvent.cs`。改字段 + 新增 struct 都要同步（v1c.3 PointerEvent 加 touch_id 漏、v1d.2 新增 KeyEvent 漏，均 final review 捕）。**教训**：csbindgen 项目 FFI struct 是「Rust 真相源 + 手补 C# 镜像」双份——「新增/改 `#[repr(C)]` struct → grep C# 镜像/补文件」是 FFI task 必检项（C# 本机不编译家里机才暴露，坑 13/38 同源）。

### 坑 36：csbindgen FFI 数组参数是 `T*` 非 managed array，须 fixed-pin（v1c.3）

**症状**：brief/草稿写 `Native.loomgui_stage_set_input(stage, events.ToArray(), events.Count)`——C# 编译错，`set_input` 签名是 `PointerEvent* events`（raw 指针）非 `PointerEvent[]`。
**根因**：csbindgen `csharp_use_function_pointer(false)`（Mono 模式）发 raw 指针参数，不自动 marshal managed array。
**解决**：`fixed (Bindings.PointerEvent* p = arr) { Native.loomgui_stage_set_input((StageHandle*)stage, p, (nuint)arr.Length); }`——`fixed` pin managed array 取 T*，调用期内钉住。空数组走 `null, 0`（Rust FFI guard）。
**教训**：csbindgen FFI 的数组/缓冲参数都是 raw `T*` + `nuint len`，C# 侧必须 `fixed`-pin（值类型数组）或 `GCHandle.Alloc`（引用类型）取指针。别直接传 managed array。`set_input`/`borrow_events`/`borrow_frame` 全是这模式。

### 坑 37：recompute 化重构漏条件门控——disabled 节点按住仍 :active（v1c review）

**症状**：PlayMode 按住 disabled 按钮仍变红（:active 触发），违反 §4.4「disabled active/click 抑制」。
**根因**：v1c.3 把 v1c.1 命令式 `set_active_chain(Some(n))`（含 `!disabled` 门控）改成全局 `recompute_active` 沿 `down_node` 链设 active，**漏复制 disabled 门控**（Down handler `down_node=hit` 无条件赋值，disabled 检查前）。更深：hit 落 disabled 节点的非 disabled Text 子（坑 29 同款挡命中）时 down_node=Text 子，原 fix 只查 down_node 漏判链上 disabled 祖先。
**解决**：`recompute_active` 链遍历**逐节点查 disabled**（不只 down_node），遇 disabled **截断**（自身+祖先都不 active）。+ 回归测（直击 / Text 子击两条）。
**教训**：命令式 set_X 重构为全局 recompute_X 时**逐条复刻原 set 的所有条件门控**（disabled/visible/touchable），重构最易丢门控。active/hover 链遍历须逐节点查状态（hit 可能落非 disabled 子，状态在祖先链）。Down+Up 同帧的测掩盖「按住 disabled」case（recompute 时 is_down 已 false）——禁用/按住类测须**分离帧**。

### 坑 38：InputSystem 1.19 `TouchPhase` 在 `UnityEngine.InputSystem` 非 LowLevel；双 using 致歧义（v1c review）

**症状**：`LoomInputCollector` 编译 CS0234（TouchPhase 不在 LowLevel）→ 改未限定后又 CS0104（ambiguous，UnityEngine.TouchPhase vs InputSystem.TouchPhase）。
**根因**：1.19 包 `TouchPhase` enum 在 `UnityEngine.InputSystem` 命名空间（Touchscreen.cs:390），非草稿/记忆的 `LowLevel`；文件同时 `using UnityEngine;`+`using UnityEngine.InputSystem;` → 未限定 TouchPhase 两命名空间都有 → 歧义。
**解决**：新/旧输入路径**全限定**——新 `UnityEngine.InputSystem.TouchPhase`，旧 `UnityEngine.TouchPhase`（不同枚举显式区分）。
**教训**：Unity InputSystem API 路径随版本变，查包源定 namespace（`Library/PackageCache/com.unity.inputsystem@*/InputSystem/Devices/Touchscreen.cs`），别信草稿/记忆。**C# 本机不编译家里机才暴露**（坑 28/35 同源）——同 using 下两 namespace 同名类型须全限定。

### 坑 39：borrow_events 的 out_len 是记录 COUNT 非 bytes（v1c.4）

**症状**：cancel_click abi_test 切 borrow_events 返回 slice 用 `len / size_of::<EventRecord>()` → `1/20=0` 记录 → 空 slice → 「Up 仍发」断言失败。
**根因**：`loomgui_stage_borrow_events(h, *mut out_len)` 写入的是 `events.len()`（记录**条数**）非字节数；brief/测草稿误以为字节。
**解决**：直接用 `len` 作记录数切 `from_raw_parts(ptr, len)`（同既有 `set_input_borrow_events_round_trip` 测用 `len * rec_size` 取字节的反向印证）。
**教训**：两个 borrow 的 out_len 语义**不同**——`borrow_events`=**记录 count**（EventRecord 条数），`borrow_frame`=**字节**（blob 字节）。测/消费代码勿混用。**复踩（v1d.4）**：v1d.4 plan T6 brief 又写 `len/size_of`——plan 作者写 FFI 测前须回读本坑。

### 坑 40：Assert.IsNotNull(ptr) 对非托管指针装箱恒非 null（no-op）（v1c.4）

**症状**：BuildStage guard `Assert.IsNotNull(stagePtr, ...)`（`stagePtr` 是 `StageHandle*`）——`stage_new` 返 null（font_path 占位未填）时 guard **仍过** → `load_html(null)` 触 FFI 崩溃，非干净测失败。
**根因**：NUnit `Assert.IsNotNull(object)` **装箱**参数；非托管指针（`T*`/`IntPtr`）装箱成含值 0 的**非 null** 对象 → 恒过。
**解决**：指针用真比较——raw `T*` 用 `Assert.IsTrue(stagePtr != null, ...)`（unsafe 上下文产 bool）；`IntPtr` 用 `Assert.AreNotEqual(IntPtr.Zero, stage, ...)`。
**教训**：NUnit Null 断言只对**引用类型**有效；值类型/裸指针/IntPtr 须显式 `!= null`/`!= IntPtr.Zero` 比较。C# 测 guard 裸指针/句柄首查此项。

### 坑 41：跨 crate 的签名变更只跑定义 crate 测会漏改消费 crate（v1d.1-T1→T5）

**症状**：T1 把 `Scene::build` 入参从 5-tuple 改 6-tuple（+draggable）。implementer 跑 `cargo test -p loomgui_core` 全绿就 commit。到 T5 首次 `cargo test -p loomgui_ffi_c` 才发现 `loomgui_ffi_c/src/lib.rs` 的 5 个 abi_test 还在构造 5-tuple → 整个 ffi crate 编不过（T1-T4 从没编译过 ffi crate）。
**根因**：`Scene::build` 是 `loomgui_core` 的公共 API，被 `loomgui_ffi_c`（abi_test 手搓 entries）+ `loomgui_core` 自身测消费。改签名只跑定义 crate 的测，漏掉跨 crate 消费者。brief 列了"修所有 5-tuple 调用点"但 implementer grep 只在 loomgui_core 内搜（lib.rs abi_test 在另一 crate）。
**解决**：签名/公共 API 变更后，跑**所有消费 crate** 的 build/test（`cargo build --workspace` 或至少 `-p loomgui_core -p loomgui_ffi_c`），不只定义 crate。T5 把漏的 5 处机械补 `, false`（FFI 测不测 drag，false 默认对）。
**教训**：workspace 多 crate 时，公共类型/函数签名变更 = 跨 crate 破坏性改动；验证须覆盖全 workspace，非单 crate。plan brief 列调用点时也别只列一个 crate。

### 坑 42：safe-area forward 渲染变换与 inverse 输入映射必须逐项一致（v1d.1-T8 Critical）
初版 forward 用 uniform `sf`、inverse 用 per-axis stretch + offX 语义错 → notched 屏触控落点≠渲染点。根因：render（root transform）和 input（ScreenToDesign）是同一 design↔screen 映射两面，必须互为精确逆。**解决**：统一 uniform `sf=min(area.w/dw,area.h/dh)` + `offX=area.x+(area.w-dw*sf)/2`（设计 span 居中 safe 区）+ `ScreenToDesign` 逐项逆 `dx=(sx-offX)/sf`；符号恒等 + notched 6 点 round-trip 测锁。**教训**：render↔input 双向映射两侧公式必须同源互逆；坐标系变换须符号推导 + round-trip 测，不能只验 degenerate case（safe==full 掩盖）。

### 坑 43：给广泛构造的 struct 加字段，plan 按文件派任务会漏枚举所有构造点（v1d.2-T1→T4）
给 `Scene` 加字段，plan 按文件派任务漏了 render/hit/layout 的字面量 → 测编译失败。根因：广泛构造的 struct 加字段是全局 fallout，plan per-file 划分枚举不全。**解决**：加字段后全仓 grep 构造点（`grep -rn "Scene {"` + `Scene::build(`）一次性枚举，不靠 per-file 任务记忆。**教训**：struct 字段/签名变更 fallout 枚举要全仓 grep 驱动，controller pre-flight grep 构造点写进 brief。**v1d.4 优化**：加相邻 transient 字段（如 `anim` 紧跟 `world_transforms`）用 `replace_all` 一次命中全 46 处字面量，cargo build missing-field 兜底。

### 坑 44：compound_matches 不检伪类 → 伪类规则污染 base_style（v1d 验收）

**症状**：v1d 加 `:focus` 后所有 `.btn` 默认紫、`.drag` 默认棕黄（v1c.1 的 :hover/:active 本也污染 base，颜色/源序未察觉，:focus 源序最后胜才暴露）。
**根因**：`pack` 调 `resolve_styles(tree, &sheet)` 用**完整 sheet**，`parse::selector::compound_matches` 只检 tag/classes/id **不检伪类标志** → `.btn:focus` 匹配 .btn 进 base（cascade specificity 同级 (0,2,0)，源序最后者胜 → 全 .btn 紫）。
**解决**：`compound_matches` 开头 `if pseudo_hover||active||disabled||focus { return false }`——伪类规则只走运行时 `rematch_pseudo_classes`，不进 base cascade。+ 回归测 `pseudo_class_rules_excluded_from_base_cascade`（RED base=紫 → GREEN base=灰）。
**教训**：base cascade（resolve_styles）与动态 rematch 是**两条独立匹配路径**；加新伪类时 `compound_matches` 跳过 + `extract_dynamic_rules` has_pseudo + `compound_matches_with_state` 状态门**三处同步**，漏任一即污染/漏匹配。

### 坑 45：键盘采集套 InputSystem 过度设计（补丁已撤销，v1d.2-T6）
CollectKeys 照搬指针 InputSystem 路径用 `Keyboard[Key]` → KeyCode≠Key 40-case 映射补丁。fix 撤销，统一 `Input.GetKeyDown(KeyCode)`（Both 模式零转换）；指针 Collect 仍双路径（多触摸要 InputSystem）。**教训**：输入采集按需选 API，键盘/鼠标按钮旧 `Input.GetKey(KeyCode)` 够，别一刀切套 InputSystem。

### 坑 46：`pub type Affine2` 与 `pub mod Affine2` 同名 type namespace 冲突（v1d.3-T1 plan self-review）

**症状**：plan 初稿想用 `pub type Affine2=[f32;6]` + `pub mod Affine2 { pub const IDENTITY... }`（测试写 `Affine2::IDENTITY`）→ 编译错「conflicting definitions」。
**根因**：Rust type namespace 同时容纳 type alias 和 module，同名冲突（value namespace 才允许 fn/const 同名 type）。
**解决**：删 `mod Affine2`，用 free fn（`pub const IDENTITY`/`pub fn from_translate`）+ `Affine2Ext` trait（链式方法），测试 `use super::*` 直接 `IDENTITY`/`from_translate(...)`。
**教训**：type alias 别想同名建 mod 提供常量；free fn + trait 是 Rust 惯用替代。plan 写代码也要 self-review 编译可行性。

### 坑 47：matrix shader 共享 material `_ObjectMatrix` 被覆盖（v1d.3 M1，**修复被坑 73① 取代**）
两非纯平移节点同 material → `SetMatrix` 最后写者胜。原 M1 fix（MPB SetMatrix）不够——MPB 不覆盖非 Properties CBUFFER（坑 73① 纠正）→ 现拆 4 Vector Properties + SetVector ×4（见坑 73）。**教训**：共享 material 下 per-instance uniform 必走 MPB（per-renderer，不污染缓存）。

### 坑 48：matrix shader GO identity 致 Mesh.bounds 剔除错位（v1d.3 I1，**补丁被坑 73③ 删除**）
非纯平移 GO identity + shader 移顶点 → Mesh.bounds 不反映世界变换 → 剔除错位。原 I1 fix（mutate bounds.center 到世界）是**补丁**：pure↔非 pure 切换双 translate 污染 mesh 资产（坑 73③）。现方案：translate 进 GO localPosition，_ObjectMatrix 只 scale/rotate → renderer.bounds 自动 world（见坑 73）。**教训**：别 mutate Mesh.bounds 做 culling（持久资产）；用 GO transform 让 bounds 自动 world。

### 坑 49：matrix shader 渲染须分顶点 re-base 两路径（v1d.3-T4 核心正确性）

**症状**：若所有节点统一 blob re-base 减 transform.x/y，非纯平移节点 world top-left≠layout top-left（旋转后偏移），re-base 错 → 顶点飞。
**根因**：blob 的 `local_x/local_y`（现 world matrix tx/ty）双用：GO 定位 + mesh re-base。identity 时两值同（layout top-left=world top-left），非 identity 时 world top-left≠layout top-left，统一减 tx,ty 对非 identity 错。
**解决**：render + blob 按纯平移分两路径——**identity**：render 产绝对顶点 + blob re-base 减 tx,ty → top-local + GO position=tx,ty（现状零改）；**非纯平移**：render 产 box 本地 (0..w)（减 layout_rect.xy）+ blob **不 re-base** + GO transform=identity + shader matrix。两处判断一致（同 `is_pure_translation`）。
**教训**：blob transform 列语义从「layout 绝对」变「world 累计」时，re-base 基准必须分路径（identity 走 layout top-left，非 identity 走 box 本地）。这是 transform 渲染最易藏 bug 处，需跨 render/mod.rs + blob.rs 一致。

### 坑 50：LoomGUIBindings.cs 是 csbindgen 自动生成 + gitignored，手写会被覆盖（v1d.4-T7）

**症状**：v1d.4 plan T7 让 implementer 手动给 `LoomGUIBindings.cs` 加 4 个 `loomgui_stage_tween*` DllImport。implementer 发现文件已被 T6 的 `cargo build` 自动再生出这 4 个，且文件 **gitignored**（`.gitignore:40 **/LoomGUI*Bindings*.cs`）——手写多余且会被下次 build 覆盖。
**根因**：`loomgui_ffi_c/build.rs` 每次 `cargo build` 跑 csbindgen 扫 `src/lib.rs` 的 `#[no_mangle]` → 写 `loomgui_unity/.../Bindings/LoomGUIBindings.cs`（best-effort，纯 Rust 构建时 Unity 目录可能不在则 `cargo:warning`）。文件是**构建产物**非源码，故 gitignored。
**解决**：新增 Rust `#[no_mangle]` FFI 符号后 `cargo build` 自动再生 C# 绑定——**绝不手编 LoomGUIBindings.cs**。C# 侧只手写**应用层镜像**（非 FFI 类型的 enum 如 TweenProp/Ease、wrapper 方法、EventType 路由）。家里机/CI 须在含 Unity 目录的仓库根 `cargo build` 才再生绑定。
**教训**：FFI 绑定是 csbindgen 单向产物（Rust→C#）；把"加 DllImport"当独立 C# 任务是错的——它是 Rust `#[no_mangle]` 任务的副产物。判断绑定文件是否手编：看 build.rs 是否 csbindgen 生成 + .gitignore 是否排除。

### 坑 51：shader matrix 路径漏 root transform（design world 当 Unity world）（v1d.3 验收）

**症状**：v1d.3 非纯平移节点（rotate/scale/skew）渲染位置/翻转/缩放全错，且与命中（design world matrix 逆投）不一致 → 点视觉位置点不到。
**根因**：shader matrix 路径 `worldPos=mul(_ObjectMatrix, box-local)` = **design world**，直接 `TransformWorldToHClip(designWorld)` 把 design 坐标当 Unity world——漏了 root GO transform（`sf,-sf,sf`+rootPos）。TRS 路径 `TransformObjectToWorld(v.pos)` 自动经 GO+root 全链，matrix 路径跳过了。
**解决**：matrix 路径 `worldPos=TransformObjectToWorld(designWorld)`（GO 是 root 子 + transform=identity → ObjectToWorld=root_ObjectToWorld，补回 design→Unity world）。两路径统一成 `root × design world`。
**教训**：core 算 design world matrix，Unity 渲染要 Unity world——桥接靠 `TransformObjectToWorld`（含 root transform）。spec §3.6 漏写这步，实现期补。坑 42（render↔input 映射一致）同类。

### 坑 52：ShaderLab 无 Matrix property + MPB 覆盖范围（v1d.3 验收，**修复方案被坑 73① 纠正为错**）
① Properties 无 Matrix 类型（只 Float/Vector/2D 等）；② MPB 只覆盖 material property。原方案「放 CBUFFER 无 Properties 对应 + MPB SetMatrix 按 name 覆盖」**错**——坑 73① 实测 MPB 不覆盖非 Properties CBUFFER 字段。现方案：拆 4 Vector Properties + SetVector ×4（见坑 73）。**教训**：MPB 只覆盖 Properties 字段；ShaderLab 无 Matrix property 类型。

### 坑 53：Unity PlayMode 首帧 Time.unscaledDeltaTime spike（tween 瞬间 complete）（v1d.4 验收）

**症状**：v1d.4 demo Start 注册 tween，Play 后 popup 无动画（opacity/scale 直接末值），dump 显示 `anim_op=1.000`（非渐变）但 complete 事件出了。
**根因**：Unity PlayMode 首帧 `Time.unscaledDeltaTime` 可达数秒（场景/library 加载延迟，实测 **dt=2.07s**）→ `advance_time(dt)` → tween `elapsed+=dt` 第一帧 `>=duration` → 瞬间 complete 写末值。dump_cdylib 固定 dt=0.15 渐变正常（证 core 对）。
**解决**：demo 改按钮/Space 触发（Play 后 dt 稳定再注册 tween）；core 不 clamp dt（破坏 stage 测 + time_s 语义，YAGNI）。
**教训**：Unity 首帧 unscaledDeltaTime spike 是通用陷阱；tween/动画别在 Start 自动播（首帧 dt 异常），用按钮/延迟/WaitEndOfFrame 触发。core time-based 逻辑（tween/longpress/双击）都对 dt spike 敏感——业务负责避开首帧。

### 坑 54：fgui `v2` 变量名误导——是 `|v|·scale` 非 `v²`（v1d.5-T6 reviewer 抓）
**症状**：scroll 惯性时长偏长 ~3x（v=2000px/s→5.5s）+ 触发阈值过敏 ~22x（v²>500 → |v|>22 即触发）。
**根因**：fgui `ScrollPane.cs:2060 UpdateTargetAndDuration` 里 `v2 = Mathf.Abs(v) * _velocityScale`（velocityScale 默认 1）——变量名 `v2` 望文生义像 v²，实为**线性 |v|·scale**。spec/plan 转录成 `v2 = v*v`（平方），duration=`log(60/v²,…)` + 阈值对 v² 判定全错。implementer 忠实 brief 无错，是 spec 转录 bug。
**解决**：`begin_inertia` 改 `let v2 = v.abs();`（duration + 阈值用线性 |v|）；`change = v*dur*0.4` 仍用 signed v 保方向。修后 v=2000→1.7s、阈值 |v|>500(PC) 合理。
**教训**：移植 fgui 算法时，**带数字后缀的变量名（v2/pos2/d2）不能望文生义**——须读源码表达式确认是平方还是线性命名残留。fgui 变量命名不一致（v2 非 v²）是通用转录陷阱。

### 坑 55：合成节点 sentinel id 跨 hit/render/wheel 须一致解码（v1d.5-T9 reviewer Critical）
**症状**：T9 改 `hit_test` 可返 sentinel thumb_id（`container|0x4000_0000`）后，T8 的 `apply_wheel_to_hit` while 循环 `scene.nodes[id.0].parent` 越界 crash（sentinel 0x4000_0000 >> nodes.len()）。T10 接 tick 后滚轮划过 scrollbar 区域必崩。
**根因**：T9 引入 sentinel id（合成 scrollbar thumb 不进 Scene 但要 hit-testable + MirrorPool 镜像），T8 的 wheel 路径读 `id.0` 索引 scene.nodes 未识别 sentinel。**跨 task 改动**：T9 改 hit_test 返回类型语义，T8 既有消费侧未同步守卫。
**解决**：`apply_wheel_to_hit` while 顶解码 `if id.0 & 0x6000_0000 != 0 { id = NodeId(id.0 & !0x6000_0000) }`（0x6000_0000=V|H flag 并集；清高位回 container_id），container 自身 effective 命中 apply_wheel。Up 段 `!grip_dragging` 守卫防 `scene.nodes[sentinel]` OOB。
**教训**：引入 sentinel/魔法 id 时，**所有读 id 做 scene 索引/沿 parent 链的路径**（hit_test/wheel/事件路由/仲裁）都须识别+解码 sentinel——跨 task 最易漏（各 task 只看自己的消费侧）。LoomGUI 首次用 sentinel id（合成节点），新路径。

### 坑 56：dirty hash 字段集遗漏致持续视觉错（v1e final review Critical）
dirty hash Mesh arm 只 hash `texture+verts.len+colors[0]` → `.btn:hover{width}` 改 size→verts 坐标变但 len/colors 不变 → hash 不变 → 误判 Unchanged（持续，非 1 帧延迟）。根因：quad 定 4 顶点，尺寸变体现在 verts 坐标非数量；Text 同族（align 改 pen_x/pen_y 但 glyph_count 不变）。**解决**：Mesh 加 verts[0]/verts[2] 首末顶点坐标 hash，Text 加首字 pen_x/pen_y，移除占位 sort_key/mask_context。**教训**：dirty hash 须覆盖体现几何变化的坐标字段（verts/glyph pen）非只 count/tex_id；字段集完整性是 final review 级审查项——per-task reviewer 易顺着 brief 验（brief 本身漏），须从"哪些视觉变化该触发重传"反推。

### 坑 57：plan/草稿写围栏外标签或属性——标签硬挡、属性静默死 CSS（v1-showcase T3/T7）
**症状**：v1-showcase plan §2 用 `<i>` 标签（justify 卡子项标记）→ 打包失败；plan §4.7 用 `position:absolute`+`left/top`（pointer-events 叠加演示）→ CSS 死代码（parse 静默忽略），reviewer 抓到。
**根因**：core parse 有 **FENCE_TAGS 硬白名单**（`parse/dom.rs`，仅 div/span/img/button/l-container）——围栏外标签 parse 失败打包报错；CSS 属性走 `style/mapping.rs` match，围栏外属性（position/left/top/z-index/background-image/font-style/grid/border-radius/渐变等）落 `_ => false` **静默忽略**（死 CSS 不报错）。v1 纯 taffy flexbox，**无 position/z-index/叠加**。
**解决**：`<i>`→`<span>`（CSS `.flx i`→`.flx span`）；删 position:absolute，pointer-events 演示改流内块 + 说明 v1 无叠加。
**教训**：写 sample HTML/CSS 前对照 FENCE_TAGS（标签）+ mapping.rs 白名单（属性）。**标签违规易发现**（打包失败），**属性违规隐蔽**（静默死 CSS，reviewer 须逐条扫 CSS 声明）。这是 AI 可预测性的打包期第一道反馈——围栏验证器工作正常。

### 坑 58：scroll offset 被 blob re-base 抵消——"控件不动仅文字动"（v1d.5 家里机验收）
**症状**：scroll 拖动时只有 Text 跟手，Mesh 控件（卡片背景/色块）纹丝不动。
**根因**：scroll offset 注入 `world_matrix.m_tx`。blob 纯平移 re-base `vert - world.tx`（坑 49）把 scroll 从 GO 挪进 vert → 渲染 `GO(world.tx)+vert(剩余)` 抵消回 layout。Text 不走 re-base（pen GO-local + GO at world.tx）所以跟随。
**解决**：render 纯平移 `rect = (wm[4],wm[5],w,h)`（world.tx 位置）非绝对 layout_rect → vert=world 位置 → re-base 减 world.tx 正好 top-local → 渲染=world.tx=layout-scroll。零回归（无 scroll 时 world.tx=layout）。
**教训**：scroll offset 与 v1d.3「绝对 vert + re-base 减 world.tx」冲突；改 scroll 进 world_matrix 后必验「Mesh 控件跟手」（不只 Text），抵消在 blob 层极隐蔽。

### 坑 59：overflow 容器被 content 撑开 + 子被 shrink——overlap=0 拖不动（v1d.5 家里机验收）
**症状**：scroll 容器 overlap=0（拖不动）—— main-scroll viewport=content=7311（被撑开）；mini `.filler{height:300}` 被 shrink 到 viewport（不溢出）。
**根因**：CSS flex §4.5 规定 overflow≠visible 的 flex item automatic min-size=0（不被 content 撑开）；LoomGUI 没设 taffy `Style.overflow`（用自己字段）→ taffy 默认 Visible → min-size=min-content → 容器被撑。同理 overflow 容器的直接空内容子（filler min-content=0）被 flex-shrink 收缩到 viewport。
**解决**：① layout build 设 `style.overflow = map(overflow_x/y)`（LoomGUI OverflowMode→taffy Overflow，Auto→Scroll）让 taffy flex automatic min=0；② build 加 `parent_overflow` 参数，overflow 容器直接子 `flex_shrink=0`。
**教训**：taffy 0.5 实现了 CSS flex §4.5（style/mod.rs:124 注释明说），但**需设 `Style.overflow` 字段触发**——LoomGUI 用自己 overflow 字段时必须同步设 taffy Style.overflow。dump_scroll 实测 overlap（别猜代码）。

### 坑 60：scroll 调试套娃——先验 layout 再改物理（v1d.5 家里机验收）
scroll 家里机拖动异常，逐层修 6 套娃 bug：drag 方向反 / x 轴 overlap=0 仍 apply delta 斜拖抖 / overflow 容器撑开（坑 59）/ 子 shrink（坑 59）/ sentinel 进 batch reorder 越界 panic / re-base 抵消（坑 58）。根因：scroll 跨 layout/render/blob/MirrorPool/merge 五层，bug 互相掩盖。**解决**：逐层 TDD + `dump_scroll` 实测 overlap 定位"哪层错"；sentinel 在 `build_render_nodes` 末尾 merge 后追加（不进 reorder）。**教训**：跨层特性 PlayMode 报「拖不动/晃动」先 example 实测 core 状态（overlap/scroll_pos/content_size）再改，避免盲改物理掩盖 layout 根因。

### 坑 61：cascade 不解析 inline `style="..."`（v0 缺口，v1-showcase 验收）
**症状**：`<div class="sw" style="background-color:#1a1d2e">` 色块透明看不见；§2 flx `style="flex-direction:column"` 被忽略（class row 兜底）。
**根因**：v0 cascade 只处理 StyleSheet rules，不解析元素 `style` attr（dom.rs attrs 收集了但 resolve 没用，cascade.rs 测试注释明写「v0 style 属性未在 dom 层解析」）。
**解决**：`css::parse_inline_style`（复用 DeclParser+RuleBodyParser，无 selector 的 declaration list）+ `cascade::resolve_styles` sheet rules 后 apply inline（specificity 最高，最后胜出）。
**教训**：inline style 是 CSS 契约，v0 缺口致 showcase 大量 `style=` 静默失效；加时复用 cssparser（手写 split 对 `url()`/注释脆弱）。

### 坑 62：Linear 项目 vertex color 没 sRGB→linear → 整体灰蒙蒙（v1-showcase 验收）
**症状**：v1-showcase 整体偏浅灰（灰蒙蒙），vs html 浏览器深蓝 dashboard；两边 letterbox 蓝（Main Camera 改 #1a1d2e）但中间 root 区灰。
**根因**：项目 Linear color space，CSS 颜色是 sRGB 编码（#1a1d2e=0.102）；Unity 不自动把 vertex color sRGB→linear → 当 linear 值 → 显示成 sRGB(0.102 linear)=0.35（浅灰蓝）。
**解决**：shader `frag` 手写 SRGBToLinear（精确 `(c<=0.04045)?c/12.92:pow((c+0.055)/1.055,2.4)`）应用于 vcolor.rgb（alpha 不转）；texture sRGB format 自动转不重复。
**教训**：Linear 项目 UI shader，vertex color（CSS sRGB）须手动 sRGB→linear；URP Color.hlsl include 路径不稳（`Couldn't open include file`），手写公式最稳。判据：背景色对但整体偏浅发灰 = color space 问题。

### 坑 63：font atlas alpha-mask（rgb 黑）→ 文字黑（v1-showcase 验收）
**症状**：坑 62 修后背景对了，但文字全黑看不清（html 是白）。
**根因**：font atlas 是 alpha-mask（字形在 alpha，rgb=黑）；shader `col=tex×vcol` 把 tex.rgb(黑)×vcol(白)=黑。core text color 正确（dump 验 #e0e0e0）传到 Unity。
**解决**：shader 加 `ALPHA_MASK` keyword（`#pragma multi_compile`，MaterialManager `program==1` 启用）：frag 分支 text=`half4(vcol.rgb, vcol.a*tex.a)`（用 vcol 色 + tex.a 字形 coverage），image=`tex*vcol`（彩色 texture rgb）。
**教训**：font atlas 单通道 alpha-mask 不能当普通 RGB texture 乘；text/image 须 shader 分支（program:1 text vs program:0 image）。诊断：TextRasterizer 加 Debug.Log 验 textColor 传对 → 黑在 shader/atlas 端。

### 坑 64：img UV v 翻转（design y-down ↔ Unity y-up，v1-showcase 验收）
**症状**：`<img>` 在 Unity 上下颠倒（text 不颠倒）。
**根因**：design y-down + LoomStage `localScale=(sf,-sf,sf)`（y-flip）→ img quad TL（design 顶）应映 texture 顶 (umin,vmax)；`mesh::quad` 固定 TL→(umin,vmin)（texture 底）→ 颠倒。text 走 TextRasterizer 用 `uvBottomLeft/uvTopLeft` 故不颠。
**解决**：render img 调 `quad(rect, white, [uv_min[0],uv_max[1]], [uv_max[0],uv_min[1]])`（swap v）；mesh::quad 不改（背景色块 UV 全图无方向）。
**教训**：img/纹理 quad UV 须配 design→Unity y-flip（TL→texture 顶）；text 独立 UV 路径不受影响。

### 坑 65：img 只设一维 → 另一维没等比（v1-showcase 验收）
**症状**：`<img style="width:48px">` 只宽变，高度没变（html 宽高一起等比）。
**根因**：layout img w/h 独立取（CSS Length > texture 原值 > 64），只设 width 时 h=texture 原 height（非等比）。
**解决**：layout img `match (w_css, h_css)`：两维都设→各自；只 width→`h=w*ih/iw`；只 height→`w=h*iw/ih`；都 auto→intrinsic。
**教训**：img 尺寸按 CSS 等比规则（只设一维按 intrinsic ratio），非两维独立取 texture 原值。

### 坑 66：改 parse-time style 逻辑必须重打 pkg（base_style 打包期烤，v1-showcase 验收）
**症状**：改 cascade（inline style 解析）后重编 .dll，色块仍不显示；重打 pkg 才对。
**根因**：`Node.base_style` 是**打包期 resolve_styles 产物**（不变，rematch 基线）；`Stage::load_package` runtime 不重 resolve（只 rematch 动态规则）。改 cascade/mapping/parse 只重编 .dll 不够。
**解决**：改 parse-time style 逻辑（cascade/resolve/mapping/parse）必须 `cargo run -p loomgui_pkg` 重打 pkg（html/css 未变也要）；纯 runtime（render/layout measure/scroll/anim）改 .dll 即可。
**教训**：分 parse-time（进 pkg base_style）vs runtime（用 pkg）逻辑；前者改重打 pkg，后者改 .dll。`dump_sw` example 验 pkg 里节点 base_style 值确认是否进包。

### 坑 67：layout/render 双测量 text 换行不一致（v1-showcase 验收，**已修·方案 A**）
showcase 72 短标题末字溢出到无高度的第 2 行。根因（推翻"浮点边界"猜测）：`measure_text` 独立调用两次，max_width 来源不同——layout（taffy 闭包）短文本只传 `None`→1 行；render 永远用 `rect.w`(stretch available) 重测，短文本 intrinsic 亚像素超 available → 误判 2 行。**解决**（方案 A，layout 为唯一测量权威）：`Scene.text_layouts` transient 字段存 layout 闭包"Some 优先"结果，render 复用（fallback `measure_text(rect.w)` 保 test 兼容）。**教训**：双测量是不一致之源；render 须复用 layout 结果非用 rect.w 重测；"浮点边界/epsilon"全是症状层猜测，dump 边界取证（`[LM] known=None` 揭示 taffy 传参）才定位真因。

### 坑 68：img Percent 压扁 + width:auto→0 不渲染（v1-showcase §1.3 验收）
两独立 bug：① **Percent 压扁**——measure 闭包对 Image 直接返 build 时 intrinsic (iw,ih) 不消费 taffy `known.width`，Percent width 图被定宽 500 但 height 用 intrinsic 64（没等比）→ 压扁；② **auto→0**——`parse_dimension` 没 handle `"auto"` 走 fallback `Length(0.0)` → rect=(0,0) 不渲染。**解决**：Image measure 存原始 `{iw,ih,w_dim,h_dim}` 闭包消费 `known` 解析（覆盖坑 65 + Percent）；`parse_dimension` 加 `"auto"→Dimension::Auto`；**改后重打 pkg**（parse-time，坑 66）。**教训**：Image measure 必须消费 taffy `known`；`width:auto` 是 CSS 默认须 Auto≠Length(0)；诊断用 `dump_img`（css.w/css.h/rect/tex 四列）+ 闭包 instrument `[IMG] known.w`。

### 坑 69：滚动松手物理（对照 fgui，v1-showcase 验收）
bug1 小拖松手回原位；bug2 快速拖到顶/底"先露空白再突然回弹"。根因：begin_inertia 硬阈值全速 inertia（界内小拖冲越界 snap）+ inertia change 未运行时截断（pos 冲远越界）。**解决**（对照 fgui）：begin_inertia 二次 ratio `((v2-thresh)/thresh)²` 削弱低速 + 越界松手直接 bounce 不 inertia；advance 运行时越界>20px 截断启回弹 tween（inertia target 不预 clamp，弹性过冲靠运行时检测）。**教训**：fgui inertia target 故意不 clamp，弹性过冲靠 RunTween 每帧检测越界>20 截断（手感来源）；初版 target-clamp 修 bug 但丢弹性（边界硬停），改运行时截断才对。fgui 行号见 §6。

### 坑 70：drag 越界双重打折致回弹弱（对照 fgui，v1-showcase 验收）
坑 69 方案 B 反馈"回弹弱"。根因：drag_follow 越界 `over=min(|np|,vp*0.5); np=-over*0.5` 双重打折（最大 vp*0.25），fgui 单打折（最大 vp*0.5）。**解决**：改 `dampened=min((lo-np)*PULL_RATIO, vp*PULL_RATIO)` 单打折，回弹翻倍。**教训**：对照 fgui 时 `min(a*c,b*c)` ≠ `min(a,b)*c`（先 cap 再打折 vs 单打折），抄公式先展开代数确认。

### 坑 71：CSS 分组选择器 `.op,.tr` 逗号不展开——规则整条失效（v1-showcase §3 验收）
**症状**：§3 .op/.tr 蓝底不显示，width/height/bg 全丢（rect 压成文字 intrinsic 尺寸）。
**根因**：parse_css 把 `.op,.tr` 整段 selector_text 喂 parse_selector，后者不认逗号 → compound class 切成 `["op,","tr"]`（逗号进 token）→ 要求元素同时含 "op," 和 "tr" → 永不匹配。
**解决**：parse_css 按逗号展开成多条 Rule（共享 declarations）；parse_selector/match_element 不感知逗号。
**教训**：CSS 分组选择器在 parse_css 层（prelude 整段切逗号）展开，别让 selector parser 处理。dump_render example 一眼定位（.tr rect 39x25 + no-bg vs 期望 80x60 + bg）。

### 坑 72：NativeHost GO 挂 root handedness flip——3D GO 被 cull（v1-showcase §1.6 验收）
**症状**：1m³ Cube 放 NativeHost 看不见 + 上下颠倒。
**根因**：LoomGUI `root.localScale=(sf,-sf,sf)` 在 transform 做 y-flip（fgui Stage `(upp,upp,upp)` 全正，y-flip 放 StageCamera position）→ GO 挂 root 子树 handedness flip（det<0）→ mesh winding 反 → Cull Back 剔除。
**解决**：建 `_container` 挂 root `localScale=(1,-1,1)` → worldScale=(sf,sf,sf) positive 翻正 handedness；GO 挂 _container + layer=LoomUILayer + material renderQueue=3000 + sortingOrder=sort_key（照 fgui GoWrapper 渲染顺序）。per-node wrapper（fgui GoWrapper cachedTransform 两层结构）保留用户 GO scale（Sync 设 wrapper 不动用户 GO）。
**教训**：LoomGUI root y-flip（vs fgui Stage 全正）致子树 handedness flip——3D GO/粒子挂 root 必 cull。独立 _container 翻正（不能改 root，UI mesh 依赖 y-flip）。照 fgui `temp/FairyGUI-unity/Assets/Scripts/Core/GoWrapper.cs`。

### 坑 73：_ObjectMatrix 三层 bug——非 pure 节点字消失（v1-showcase 按钮验收）
**症状**：按钮 :active{transform:scale} 字消失（按下/松手切换 pure↔非 pure 触发；按住显示松手消失循环）。
**根因**（三层叠加）：① `_ObjectMatrix` 在 CBUFFER 无 Properties 对应 → MPB.SetMatrix 不覆盖（**纠正坑 52**：非 Properties CBUFFER 字段 MPB 也不覆盖，坑 52「放 CBUFFER MPB 覆盖」错）；② 拆 4 Vector 后 HLSL `float4x4(v0..v3)` 是 **row-major**（v0..3=行），MirrorPool `GetColumn`（列）错位 → Mtx/Mty 没进 x/y（跑 w 分量）+ Mb/Mc 错位；③ I1 fix（坑 48）mutate `Mesh.bounds.center` 做 culling，非 pure→pure 切回时 GO localPosition=(Mtx,Mty) + mesh bounds（含旧 Mtx）= 双 translate → frustum culling 误剔（bounds 跑到 design y≈2630 超视口）。
**解决**：① `_ObjectMatrix` 拆 4 Vector Properties + MPB SetVector ×4；② `GetRow`（配 HLSL row-major）；③ 删 I1 fix，非 pure 也 GO localPosition=(Mtx,Mty)（translate 进 GO），_ObjectMatrix 只 scale/rotate → renderer.bounds 自动 world（culling 正确）。
**教训**：MPB 只覆盖 Properties 字段（坑 52 需纠正）；HLSL `float4x4(v0..v3)` row-major 不是 column；**别 mutate Mesh.bounds 做 culling**（mesh 持久资产，pure↔非 pure 切换污染）——用 GO transform 让 renderer.bounds 自动 world。

### 坑 74：ResolvedStyle 加字段必 bump PKG_FORMAT_VERSION（v1.1 background-image）
**症状**：spec 写"零改 FFI/blob"，但 Task 1 给 ResolvedStyle 加 `background_image`/`background_size` 两字段后，最终审查发现 `PKG_FORMAT_VERSION` 没 bump。
**根因**：`ResolvedStyle` 经 `bincode::serialize(&n.style)` 进 pkg.bin 每节点（`asset/mod.rs:188`）——bincode 是**字段序定长无 schema**，struct 中段加字段改变每节点 style blob 字节布局。版本仍 7 → 旧 v7 pkg 过 version gate 但反序列化 garbage（不 panic 即静默坏）。
**解决**：加 ResolvedStyle 字段 = bump `PKG_FORMAT_VERSION`（7→8）+ MIN/MAX（约定"拒绝+重打无迁移"，v5→6→7 同）。更新 `read_rejects_unsupported_version` 测试基线。C# 不解析 pkg（Rust-internal）零改。
**教训**：spec 写"零改 blob"前先查 ResolvedStyle 是否 pkg 载荷——**它是**。任何进 pkg.bin 的结构（ResolvedStyle/DynamicRuleTable）加字段必 bump version。设计阶段就该标出，别等最终审查。

### 坑 75：dirty hash 跳 uvs → background-size 单独变 stale（v1.1 background-image）
**症状**：`:hover` 切 `background-size: 100%`→`cover`（同纹理）渲染不更新。
**根因**：`dirty.rs:29` Mesh hash 解构 `Mesh { texture, verts, colors, .. }` 用 `..` 跳 `uvs`。background-size 变 → `fit_uv` 重算 UV，但 texture/verts/colors 不变 → hash 不变 → Unchanged → 新 UV 不 emit。Task 4 只验 background_image 变（改 texture→hash 变）漏了 size 单独变。
**解决**：destructure 加 `uvs` + hash `uvs[0]`/`uvs[2]`（同 verts 首末摘要模式）+ `mesh_uv_change_changes_hash` 测试。
**教训**：dirty hash 摘要要覆盖**所有渲染可见字段**。Mesh 的 `uvs` 是渲染可见的（UV 变=采样区变），`..` 跳过=漏。加新渲染路径（fit_uv）时审查 dirty hash 是否捕到其产物。

### 坑 76：dirty hash 采样首末顶点 → 圆角 mesh 非首末角变 stale（v1.2 border-radius）
**症状**：仅 BL 角非零且 BL 半径变（如 `border-bottom-left-radius:4px`→`5px`，TL/TR/BR 全 0，sides 不变）→ 渲染不更新（stale Unchanged）。
**根因**：§2.24 dirty hash 对 Mesh 只采样 `verts[0]`(中心)/`verts[2]`(TL 第二弧顶点)。圆角 mesh 25 顶点时 `verts[0]`=矩形中心（不随半径变），`verts[2]` 只反映 TL 角——BL-only 变 + sides 不变 → `verts.len` 也不变 → hash 不变。坑 75 修了"跳 uvs"，但采样摘要在多顶点 mesh 上仍漏非首末顶点。最终 opus 审查抓出（spec §9.1 要求的 `radius_0_to_8_hash_changes` 回归测试 plan 漏写）。
**解决**：`verts.len()>4` 时 hash **全顶点/全 UV**（圆角 mesh ≤33 顶点，成本可忽略）；`<=4`(quad) 保留首末采样（零回归 hash 不变）。加 `bl_radius_4_to_5_hash_changes` 测试（同 verts.len，仅 BL 半径变 → hash 必变）。注意 colors 哈希位置在 if/else 前（保 quad 流顺序零回归，re-review 抓的 Important）。
**教训**：dirty hash 采样摘要（首末/N 个）只对**固定顶点数** mesh 安全；变顶点数 mesh（圆角/多边形）须 hash 全顶点。摘要策略要随 mesh 类型分档，不能一套首末通吃。

### 坑 77：rounded_rect 混合椭圆角直角分支落圆心+方向偏离矩形顶点（v1.2 家里机验收）
**症状**：`border-radius:0 8px / 8px`（TL/BR 水平半径 0 → 直角，TR/BL 真弧）→ TL/BR 角附近镂空。
**根因**：直角分支 `corner_pt = 圆心 + (cos(start)·rx, sin(start)·ry)`。rx=0 ry>0 时 py=圆心.y+sin·ry 偏离矩形顶点（TL 落 [0,8] 非 [0,0]）。spec §5.1 注已预见"实现时可更直白按角序硬编码四角顶点"，但实现者照 brief 代码用了圆心+方向法。
**解决**：corners 元组附矩形顶点 corner（TL=[x,y]/TR=[x+w,y]/BR=[x+w,y+h]/BL=[x,y+h]），直角分支直接 push corner，不依赖圆心+方向。
**教训**：spec 注预见的简化方案别跳过——圆心+方向法在 rx=ry 时巧合正确（ry=0 不偏移），混合椭圆角（一轴 0 一轴 >0）才暴露。几何分支的退化 case（某半径 0）须显式落已知点，别靠通用公式。家里机实机验收才抓到（单测用对称半径漏）。

### 坑 78：scroll + CLIPPED clip rect design/world 空间错位全裁（v1.1/v1.2 家里机验收）
**症状**：`overflow:hidden` 容器（bg-demo/br-demo）scroll 时内容全空（col.a=0 全裁）。
**根因**：`transform.rs` 给子节点 world_matrix 注入 `T(-祖先.scroll_pos)`，节点 world 在 `(layout - scroll_offset)` 空间。但 CLIPPED 的 `_ClipBox` 由 `ComputeClipBox(root, design_rect)` 算（design 空间，不含 scroll），shader `clipPos = worldPos.xy × _ClipBox.zw + _ClipBox.xy` 用 world（含 scroll）——空间错位 → scroll > clip 半边时 clipPos 超界 → `step(max,1)=0` → col.a=0 全裁。scroll 越深越明显。
**解决**：`batch.rs::assign_sort_keys` dfs 加 `scroll_offset` 累积参数。**own clip 减 scroll_offset**（本节点 world 在 layout-scroll_offset 空间，clip rect 同空间）；**accumulated 不减 scroll**（祖先 clip 如 scroll 容器 viewport 在 world 固定——容器自身 world 不含自己 scroll_pos）；`intersected = accumulated(design) ∩ own(world)` = world 可见区。v1 错修（accumulated 也减 → viewport 跟着移，design 不相交同减仍 h=0），v2 才对。
**教训**：clip rect 空间必须与 shader clipPos 空间一致（都 world 或都 design）。scroll 容器 viewport 在 world 固定（容器自身 world 不含自己 scroll_pos），子节点 world 含 scroll——两者求交须先把 own 转 world。家里机用 `dump_clip_scroll` 实机 dump（先 tick 建 scroll 表再 set_scroll_pos）才发现 v1 错——本机单测测不出（无 scroll 运行时）。

### 坑 79：shader `tex×vcol` 非 CSS 合成 → 图透明区全透明不透 bg-color（v1.1 spec §6.2 承诺错误）
**症状**：§3.6 home.png（透明背景 icon）+ bg-color 共存，图透明区透出 root 深蓝，透不出 bg-color（青/红底看不见）。
**根因**：shader `LoomGUI/Unlit` program:0 frag `col = tex * vcol` = `(tex.rgb×vcol.rgb, tex.a×vcol.a)`。图透明区 `tex.a=0 → col.a=0` 全透明——是简单 tint，**非 CSS background 合成**（图透明区应透 bg-color）。v1.1 spec §6.2 承诺"shader mainTexture×vertexColor：图透明区透出 background-color"是**数学错误**（tex×vcol 在透明区得 0，透不出 vcol）。
**解决**（方案 A，待实现）：img 保持 program:0（tex×vcol，图透明透下层）；Container+bg-image 用 program:2 + `BG_COMPOSITE` keyword 走真合成 `col.rgb=tex.rgb×tex.a + vcol.rgb×(1-tex.a); col.a=vcol.a`。program 须进 frame blob（VERSION 4→5）——因 img 和 Container+bg-image 都用 tex1，shader 靠 program 区分。详见 `docs/superpowers/plans/2026-06-30-bg-image-composite.md`。
**教训**：shader 乘法 tint（tex×vcol）≠ CSS 合成——透明区是 0×vcol=0（透明）非透 vcol。色图共存须加法合成（tex.rgb×tex.a + vcol.rgb×(1-tex.a)）。spec 写"shader 已支持共存"前先算 shader 数学，别假设。img vs Container+bg-image 共用纹理时 shader 须靠 program 分流。

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
- **scroll overlap 实测**（v1d.5，坑 60）：写 `examples/dump_scroll.rs`（`load_package` + `tick_and_render` + 遍历 `scene.scroll.0` dump content/viewport/overlap/scroll_pos）。PlayMode 报「拖不动/晃动」先实测 core overlap——overlap=0 是 layout 问题（overflow 撑开/子 shrink，坑 59），非物理（方向/惯性，坑 58/60）。别猜代码。
- 查 crate 实际 API：`~/.cargo/registry/src/<crate>-<ver>/src/`。
- **PlayMode 命中诊断**（v1c.1）：Unity 侧加 diag log——`LoomInputCollector.Collect` Down 时 log `design=(dx,dy)` + `LoomStage.LateUpdate` log `mouse/screen/evLen/onUI`。core 侧独立验：写临时 `examples/dump_xxx.rs` 跑 `Stage::tick_and_render` 后 `hit_test(scene, design_pt)`。**core 命中但 PlayMode onUI=false → 坐标映射或 set_input 传输问题**（如输入系统不匹配坑 28）；**core 也不命中 → AABB/坐标换算**。例：坑 29 诊断时 core `hit_test(270,46)→Some(btn1)` 但 PlayMode onUI=false，定位到 Collect 未被调（LoomInputCollector 组件没挂 GO）。
- **命中 y 偏移诊断**（v1c.1）：「按钮下半段响应上半段不响应」= Text 子节点 AABB 盖父上半段（`<div>文字</div>` 自动 Text 子 `layout_rect` 与父重叠上半）→ 逆等效序命中 Text 而非父。dump 节点 AABB 确认子是否盖父 + 对照坑 29（hover 祖先链）根治。
- Rust→Unity 闭环：改 Rust 后 `cargo build -p loomgui_ffi_c --release` → 关 Unity → `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/`。
- Unity 验证：Test Runner EditMode（`Window→General→Test Runner`）；PlayMode 看 Game 视图渲染；PlayMode 前确认 `.dll` 是最新版。
- 跨语言 round-trip：Rust `build_blob` ↔ C# `FrameBlob` 靠手搓 blob byte[] 的 EditMode 测互验（blob 布局是 Rust↔C# 契约，两端须字节级一致；改列/偏移必同步）。**手搓多节点 fixture 必 SOA 列优先**（坑 12，单节点掩盖 AoS 错）。
- **bump blob version 清单**（v1b.2，坑 17）：① Rust `blob.rs` VERSION+COLUMNS+`num_col_offsets=columns.len()`（自动传播 header_len）；② C# `FrameBlob.cs` `ExpectedVersion` + 所有 arena offset 基准（`12+14*4` 非 `13*4`）；③ grep C# 测目录**所有** builder：`version=Nu`/`HeaderLen`/`elemSize = {`/`i < N`/末列写——逐个升，header 列数==data 列数；④ 重编+关 Unity 换 .dll。
- **stale .dll 诊断**（v1b.2）：PlayMode 全不渲 + Console 干净 → `md5sum` 对比 fresh release .dll vs `Assets/Plugins/LoomGUI/`，committed .dll 应==release（坑 10）；committed .dll md5 记在 progress ledger 便于核对。
- **stale .dll 诊断**：PlayMode **全不渲 + Console 干净** → `md5sum target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/loomgui_ffi_c.dll`，不等 = stale（Rust 改 blob/ABI 格式没换 .dll，坑 10）。
- **stale .dll 行为验证**（坑 10 强化）：md5 对比只证文件版本，**行为验证**用 `libloading` 加载 Plugins .dll 跑 FFI（临时 `[dev-dependencies] libloading` + example 调 stage_new/tween/tick/dump_scene）对比 rlib example——cdylib 和 rlib 同源码同行为，若 rlib 对但 Unity 错 = **Unity 加载/调用问题**（md5 一致仍 stale，如 Unity 进程没重启加载旧 dll）。区分「.dll 坏」vs「Unity 没加载 fresh .dll」。
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
- **dump_text 双测量对比**（v1-showcase 验收，坑 67 方案 A）：`examples/dump_text.rs` 加载 showcase pkg + CJK 字体（wqy-microhei.ttc），tick_and_render 后对每 Text 节点对比 **before**=`measure_text(Some(rect.w)).lines`（修复前 render 行为）vs **after**=`scene.text_layouts[node].lines`（修复后 render 复用）。修复前 72 短标题 before=2/after=1（bug），长文本 before=after 多行（不误伤）。**定位 taffy 传参**（关键取证）：临时在 layout measure 闭包加 `eprintln!("[LM] {:<16} known={:?} ln={}", content, known.width, layout.lines.len())`，揭示短文本 taffy 只传 `None`（判 max-content ≤ available）、长文本传 `Some(available)`——这是「render 不能用 rect.w 作 max_width」的根因证据。line_height 传 **0.0**（默认 em）。
- **dump_img img 布局诊断**（坑 68）：`examples/dump_img.rs` 加载 showcase pkg，dump 每 Image 节点的 `css.w/css.h`（Dimension 枚举）/`rect`/`tex(iw,ih)` 四列——一眼定位 Percent 压扁（rect.w:rect.h ≠ iw:ih）/ auto→Length(0)（css.w 显示 `Length(0.0)`）/ 未注册（UNREGISTERED）。配合 layout 闭包 instrument `[IMG] known.w={:?}` 揭示 taffy 对 Percent width 第二次传 `known.width=Some(解析宽)`。
- **chrome MCP 验 html 契约**（v1-showcase 验收）：showcase html 不能直接浏览器开（div 默认 block，css 没写 display:flex，flex-direction 无效）→ index.html 加 `<head>` 预览覆盖（`div{display:flex;flex-direction:column}` 复刻 LoomGUI「div 永远 flex」契约 + `*{box-sizing:border-box}` 复刻 taffy 默认 border-box），scraper select body 不读 head → pkg 不变。chrome-devtools MCP `evaluate_script` 读容器 computed style + 子元素 rect（同 y 横排/同 x 竖排）判断渲染，比截图精确。
- **红色实验验渲染**（v1-showcase 验收）：怀疑某节点没渲染时，临时改其 bg 为红色（style.css + 重打 pkg）→ 重进 PlayMode 看是否变红。红=节点渲染正常（"灰"是颜色空间/对比度/感知），灰=没渲染（GO/material/camera/未进 render tree）。core 端 `dump_sw` example 验 pkg 里节点 base_style/bg 值确认数据对。
- **改 parse-time 逻辑要重打 pkg**（坑 66）：改 cascade/resolve/mapping/parse 后重编 .dll 不够——`base_style` 是打包期产物，`cargo run -p loomgui_pkg` 重打 pkg 才进包（html/css 未变也要）。判据：runtime 用 pkg 的逻辑改 .dll，parse-time 进 pkg 的逻辑改要重打。
- **fgui ScrollPane 物理参照行号**（坑 69/70）：`temp/FairyGUI-unity/Assets/Scripts/UI/ScrollPane.cs`（2320 行）。松手物理：`__touchEnd:1610`（越界 flag→直接 bounce / 界内→inertia）、`UpdateTargetAndDuration:2048`（二次 ratio `((v2-thresh)/thresh)²` 削弱低速 + `dur=log(60/v2_eff)/log(decel)`，坑 54 `v2=|v|·scale` 非 v²）、`RunTween:2245`（cubic_out 推进 + 运行时越界>20 截断启回弹 tween = 弹性过冲）、`__touchMove:1430`（drag 越界打折 `min(位移*0.5, vp*PULL_RATIO)`）。常量：`PULL_RATIO=0.5`(:90)、`TWEEN_TIME_DEFAULT=0.3`(:89)、过冲阈值 20（RunTween 硬编码 `>20+threshold`）。**fgui pos 负**（`container.y∈[-overlap,0]`），LoomGUI `scroll_pos` 正（`[0,overlap]`）——对照时符号反转。
- **dump_render 渲染节点 payload**（坑 71/73 诊断）：`examples/dump_render.rs` 加载 showcase（inline 读 html/css 或 pkg）+ tick_and_render，遍历 frame.nodes dump `node_id/classes/rect/payload kind(Mesh/Text/Unchanged)/colors[0]/style.bg`。一眼定位：bg 缺失（no-bg + Mesh c0 透明）/ 尺寸错（rect 39x25 vs 期望 80x60）/ 逗号规则失效（.tr 应有 bg 但 no-bg）。**inline 模式验 parse 修复**（绕过旧 pkg 缓存，不需重打）。
- **dump_interact 交互帧 sequence**（坑 73 诊断）：`examples/dump_interact.rs` 构造最小复现（btn+Text+`:active{transform:scale}`）→ 打包伪类 → load → tick(首帧) → set_input Down/Up + tick×N → 逐帧 dump btn+Text 子 `payload kind + world_matrix`。定位 Unchanged/re-emit 切换 + world 进 transform 的帧延迟（compute_world_transforms 在 rematch 前，transform 次帧才进 world）+ pure↔非 pure 路径切换。core emit 正常（frame5 Text re-emit Text L1）→ bug 在 Unity。
- **fgui GoWrapper 参照行号**（坑 72）：`temp/FairyGUI-unity/Assets/Scripts/Core/GoWrapper.cs`。渲染顺序 3 机制：`SetWrapTarget:94`（GO SetParent cachedTransform）、`SetGoLayers:113`（GO+子 layer=UI 层，UI 相机渲染）、`CacheRenderers:167`（material renderQueue=3000 Transparent）、`SetRenderingOrder:261`（GO sortingOrder=GoWrapper renderingOrder）。**fgui Stage `(upp,upp,upp)` 全正 scale**（Stage.cs:252/935，y-flip 放 StageCamera.cs:115 position y 负）—— vs LoomGUI root `(sf,-sf,sf)` transform 做 y-flip，是 NativeHost handedness bug 根因。
- **subagent 并行盘点/清理**：注释精简等机械大范围清理派多 subagent 各负责一组文件（注释精简幂等——分类器中断重派基于当前状态安全）；并行 subagent **不各自跑 cargo**（workspace 锁冲突），controller 最后统一 `cargo test --workspace` + `git diff` 抽查兜底。
- **dump_clip_scroll 诊断 clip+scroll**（坑 78）：scroll+CLIPPED 全裁时写 `examples/dump_clip_scroll.rs`——`load_package` + **先 `tick` 一次建 scroll 表**（overflow:scroll 节点入表）+ `set_scroll_pos` + 再 `tick` + 遍历 `frame.clips` dump `context_id/rect`。对照预期 world 可见区（own layout 减 scroll_offset，被祖先 viewport 裁）。**set_scroll_pos 须在 tick 后**（tick 前 scroll 表空 → no-op，坑 78 踩）；PlayMode 用户拖动时表已建不受影响。
- **FFI blob 二分排查**（坑 78 诊断 §3.7 圆角不显示）：showcase 视觉坏时先补 FFI blob round-trip 测试（构造目标 mesh——如 25-vert 圆角——`build_blob` 序列化 + TestView 反序列化验 vert_count/idx/re-base 保真）。通过 → bug 在 Unity 侧（.dll 版本/环境/shader），非核心序列化；失败 → 核心 blob 链问题。比直接猜 Unity 侧快。
- **SDD brief 值偏离防护**（v1.2 T7 教训）：showcase 卡等"值敏感"任务，haiku 实现者照 brief 代码时易自行调值（v1.1 §3.6 / v1.2 §3.7 都发生：背景色挪模板统一、椭圆角 `20px/10px` 拆两行丢 `/` 语义）。controller 收尾须 `git diff` 逐行对照 brief 值，偏离即修回。dispatch 时明示"严格用 brief 值 verbatim，勿调"仍不够——机械模型易"优化"。值敏感任务用 sonnet 或 controller 直接写。

## 7. 已知问题/未完成（v0 ledger）

> **验收文档已清理**：v1c.2-v1d.2 的 `docs/vX.Y-home-verification.md`（家里机验收清单）验完即删——验证场景已内联在各版本 ✅ ledger 行（EditMode/PlayMode 要点），历史文档可 `git log` 追。后续版本不再 commit 此类一次性清单。

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

**v1c.2 defer**：~~CaptureTouch/多触摸槽（v1c.3）~~（v1c.3 ✅）、~~click downTargets 链兜底+双击+Move 超阈值取消+Canceled+CancelClick~~（v1c.4 ✅）、~~stopImmediatePropagation~~（v1c.4 ✅）、broadcast 子树广播（v1.x）、onKeyDown/Up/onMouseWheel 路由（v1d+）、AncestorChain 池化（Move 热路径，fgui 池 callChain，v1c.2 YAGNI 未池）、transform world_to_local 命中（v1d）。

**v1c.3 ✅ 完成（main @ 2beb6e7，待家里机验）**：多触摸 + CaptureTouch——核心 `PointerState` 单指针 → 5 槽 `TouchSlot`（slot0 鼠标 -1 + slot1-4 触摸，鼠标+触摸共存）+ EventRecord/PointerEvent 加 `touch_id`（EventRecord 20B/PointerEvent 16B，**PointerKind `repr(u8)`** 坑34）+ active/hovered 全局 union recompute（删 set_*_chain，修 v1c.2 多指互清 bug）+ CaptureTouch/touch monitor（照 fgui 消费即清，cap/bub 各加一，add/remove_touch_monitor FFI）+ Move 对齐 fgui（无 monitor 不产，v1c.2 鼠标 Move 沿链 bubble 行为废止）+ Up 去重 + Unity `LoomInputCollector` 多指采集（fixed-pin 坑36）。click 沿用 v1c.2。**frame blob/MirrorPool/.pkg.bin v3 零改**（EventRecord 不进 blob）。spec `docs/superpowers/specs/2026-06-23-v1c.3-multi-touch-design.md`、plan `docs/superpowers/plans/2026-06-23-v1c.3-multi-touch.md`、验收 `docs/v1c.3-home-verification.md`。**两机约束**：core+ffi `cargo test --workspace` 180 测全绿；C# 本机写未编译（无 Unity），家里机 EditMode（capture 测骨架补 handle）+ PlayMode（多指/capture/鼠标回归）。subagent-driven 6 task 全 Approved + final review Ready（opus）。踩坑：PointerKind repr(C) 4B（坑 34）、csbindgen 不生 use-imported struct stub 手补镜像漏（坑 35）、set_input PointerEvent* 须 fixed-pin（坑 36）。brainstorm 阶段 subagent 审核 + 二次 fgui 实证核实修正 4 个 Critical（touch_id 哨兵撤销/capture cap+bub/Up 去重/active 双写）。

**v1c.3 defer（→ v1c.4 click 增强，正交）**：双击（350ms 窗口+位置+同键）、downTargets 链兜底（down 目标被移除沿祖先找，照 fgui ClickTest）、缩放容忍、Move 中超阈值取消 click、Canceled 跳过 click、Stationary hover 跟随（元素动后刷新，照 fgui 局限）。其余同 v1c.2 defer（stopImmediate/broadcast/键盘/transform 命中）。

**v1c review ✅ 修复（main @ e262d26）**：review v1c.2/.3 发现并修 3 类——① disabled-active 回归（§4.4，recompute_active 漏 disabled 门控 + Text 子击中漏判祖先链，坑 37，链遍历逐节点查 disabled 截断）；② v1c.2/.3 C# 编译错（C# 本机不编译家里机暴露）：`TouchPhase` 1.19 命名空间+歧义（坑 38，双路径全限定）、`EventContext.Get/Return` internal→public（坑 33 同模式重现）；③ `find_node_by_id` enabler（`Scene::find_by_id_attr`+FFI+C# 包装，替代硬编码 build-id）+ interact 禁用按钮加 id + demo find+set_node_disabled（原仅 CSS opacity 视觉、Node.disabled 未设）。.dll 重编 + pkg 重打。**PlayMode 验**：按住禁用按钮不变红（含 Text 子击中）。core 150+ffi 25 测全绿。**C# 路由测仍多 Assert.Ignore 骨架**（家里机手验未回填真断言）——v1c.4 前可补 BuildStage helper un-ignore。

**v1c.4 ✅ 完成（main @ db33b24，待家里机验）**：click 全对齐 fgui + v1c 收尾——core `click_test`（Click 目标=down_targets[0] 按下叶非当前 hit，照 fgui ClickTest；per-axis 阈值 mouse10/touch50 固定像素无缩放；down_leaf 失效沿祖先兜底）+ 双击 clickCount 1→2→1（350ms+per-axis+同键，`bump_click_count`；EventRecord pad[0]→`click_count:u8` offset5 20B 不变）+ Move>50 取消（硬编码）+ **Canceled**（`PointerKind::Canceled=3`，偏离 fgui quirk=隐式 CancelClick：置 click_cancelled→不发 Click+reset，spec §0.6 用户确认）+ CancelClick API（`cancel_click` FFI + C# CancelTouch）+ **stopImmediatePropagation**（纯 C# W3C，EventBridge GetInvocationList 逐回调 break null-safe）+ **Stationary hover 跟随**（v1c.3 defer 项落地：process 头部无事件活跃槽 re-hit-test，fgui 改进）+ time_s 复用 tick(dt)（C# unscaledDeltaTime → Stage::advance_time，零新 FFI 参数）。**frame blob/MirrorPool/.pkg.bin 零改**（EventRecord 不进 blob，click_count 复用 pad）。spec `docs/superpowers/specs/2026-06-24-v1c.4-click-design.md`、plan `docs/superpowers/plans/2026-06-24-v1c.4-click.md`、验收 `docs/v1c.4-home-verification.md`。**两机约束**：core+ffi cargo test 全绿（core 162+ffi 28）；C# 本机写未编译（无 Unity），家里机 EditMode（16 测，**BuildStage helper 须填 font_path**，否则 stagePtr!=null guard 失败）+ PlayMode（双击 isDoubleClick / 拖拽>50 取消 / 触摸 Canceled / stationary hover / CancelTouch / StopImmediate）。subagent-driven 8 task 全 Approved + final review Ready（opus，6 跨 task 集成点全清）。踩坑：borrow_events out_len 是 count 非 bytes（坑 39）、Assert.IsNotNull 对指针装箱 no-op（坑 40）。

**v1c.4 defer（→ v1d+）**：broadcast 子树广播（v1.x）、onKeyDown/Up/onMouseWheel 路由（v1d+）、transform world_to_local 命中（v1d）、AncestorChain 池化（Move 热路径，v1e perf）、~~长按 onLongPress（fgui holdTime/downFrame）~~（v1d.1 ✅ universal longpress）、invalidation set 伪类重匹配优化（v1e 撞墙）。click 已全 fgui 对齐（缩放容忍证伪删除）。

**v1d 路线固化（main @ ea38bd5）**：`docs/roadmap/v1d-plan.md`——v1d = 各轮 spec 标 v1d/v1d+ 全量项照单全收（治 defer-defer 遗失），12 项 5 子轮：v1d.1 拖拽+长按+safe-area / v1d.2 键盘+焦点 / v1d.3 transform / v1d.4 GTween / v1d.5 ScrollPane+滚轮+手势仲裁。仅 v1d.5←v1d.1 依赖；IME 默认 defer 随 TextInput（v1.x）；broadcast/AncestorChain/invalidation 不进 v1d（v1.x/v1e）。关 v1 验收 #2 可滚动容器 + #4 safe-area。

**v1d.1 ✅ 完成（main @ 0fac306，待家里机验）**：drag + longpress + safe-area，检测全 core（input.rs），机制镜像 fgui（已核实源码）。**drag**（opt-in `Node.draggable` HTML 属性，pkg v4 NodeBlock flags bit0 formatVersion 3→4；core 状态机阈值 mouse2/touch10 per-axis < click 10/50；drag_target=down_targets.iter().find 内层 draggable；DragStart 置 click_cancelled；EVT 6/7/8；只发事件不跟手→v1d.3）+ **longpress**（universal，tick 1.5s/50px，process 头部空事件也跑；EVT 9；与 click 独立）+ **safe-area**（纯 Unity 侧 LoomStage Screen.safeArea shrink-to-fit + ScreenToDesign 逐项逆，core 零改；T8 一轮 Critical fix：offX 设计 span 居中非屏幕中心偏移 + forward/inverse 同源互逆，坑 42）。EventRecord 仍 20B（复用 event_type 6-9）；version v1d.1；无新 FFI（csbindgen 不 regen）；.dll 重编（1694720B）。spec `docs/superpowers/specs/2026-06-24-v1d.1-drag-longpress-safearea-design.md`、plan `docs/superpowers/plans/2026-06-24-v1d.1-drag-longpress-safearea.md`、验收 `docs/v1d.1-home-verification.md`。**两机约束**：core+ffi cargo test 全绿（core 185+ffi 30=215）；C# 本机写未编译，家里机 EditMode（LoomEventHandlerTests 18 含 T7 drag/longpress bubble + LoomInputCollectorTests 含 T8 NotchedSafeArea_RoundTrip 6 点硬门，BuildStage helper 须填 font_path）+ PlayMode（drag opt-in/取消 click/阈值 + longpress 1.5s 一次/Move>50 取消/独立 click + safe-area Device Simulator 避刘海/触控↔渲染对齐/关 _safeArea 回归）。subagent-driven 8 task 全 Approved（T8 一轮 Critical fix）+ final review Ready（opus，8 跨 task 集成点全清）。踩坑：跨 crate 签名变更漏改（坑 41）、safe-area forward/inverse 变换不一致（坑 42）。**M6 行为变化**：设计 aspect≠屏 aspect 时 v1c stretch → v1d.1 letterbox 居中（spec §5.1 本意，修 v1c latent 不一致）。

**v1d.1 defer（→ v1d.2+）**：drag 跟手（自动移动节点，→v1d.3 transform.translate 启用）、longpress onBegin/onAction 重复/onEnd（v1x 全套手势）、CSS env(safe-area-inset-*) per-element 内边距（v1x）、DragDropManager DnD（v1x）、swipe/pinch/pan 手势（v1x）、键盘/焦点/Tab/滚轮/IME（v1d.2/v1d.5）、I1 safeArea 变不重调（v1d.x，ConfigureTransforms 仅 Screen.width/height 变时重调）。

**v1d.2 ✅ 完成（main @ 8f2629e，待家里机验）**：键盘 keydown/up + 焦点 + Tab 导航 + `:focus` 伪类 + FocusIn/Out，检测全 core（input.rs），机制镜像 fgui（已核实源码）。**焦点**（单一全局 `Scene.focused_node` + `Node.focused`/`Node.tabindex` HTML 属性 None/-1/0/N；pkg v4→**v5** NodeBlock +tabindex i32 i32::MIN 哨兵，MIN=MAX=5 拒 v4；`Scene::build` 6→7-tuple）+ **focus 通道单源** `focus_node`（3 处共用：Stage tick 消费 pending_focus_request / Down arm click-to-focus tabindex>=0 / process_keys Tab）+ **request_focus 强制**（任意非 disabled 节点，照 fgui RequestFocus 不查 focusable；不直写 last_events 记 pending 下 tick 消费）+ **Tab 链**（正整数升序 stable 后 0 组 DFS，-1/None/disabled 排除；Tab/Shift+Tab wrap；**Tab 消费不发 keydown**）+ **keydown/up**（新 `KeyEvent` 8B 输入，有焦点才发，复用 EventRecord 流 EVT 12-15 touch_id 装 key_code/pad[0]=modifiers，零新输出 ABI；无焦点丢弃）+ **`:focus`**（Compound.pseudo_focus dynamic.rs 单一定义 + 门控 node.focused + extract_dynamic 纳入，rematch 每帧吃）。tick 管线 +⓪pending_focus_request +②process_keys。version v1d.2；3 新常驻 FFI（set_key_input/request_focus/focused_node）+ KeyEvent struct；.dll 重编（1694720→1709056B）。spec `docs/superpowers/specs/2026-06-24-v1d.2-keyboard-focus-design.md`、plan `docs/superpowers/plans/2026-06-24-v1d.2-keyboard-focus.md`、验收 `docs/v1d.2-home-verification.md`。**两机约束**：core+ffi cargo test 全绿（core 206+ffi 35=241）；C# 本机写未编译，家里机 EditMode（LoomEventHandlerTests 20 含 T7 KeyDown/FocusIn bubble，BuildStage helper 须填 font_path）+ PlayMode（tabindex opt-in/click-to-focus/Tab wrap/keydown 需焦点+Tab 消费不发/:focus/request_focus 下 tick 生效）。subagent-driven 7 task 全 Approved + final review Ready（opus，11 跨 task seam 10 清 + C1 Critical 修）。踩坑：csbindgen struct stub 复发（坑 35 强化—新增 FFI struct 必补镜像）、Scene 加字段 plan 漏枚举构造点（坑 43）。

**v1d.2 defer（→ v1d.3+ / v1.x）**：IME/character（随 TextInput v1.x）、TextInput 控件（v1.x）、`:focus-visible`（区分键盘/鼠标聚焦成因，v1.x）、Tab preventDefault（业务拦 Tab，v1.x）、Tab 链缓存（每 Tab O(N) 即时构造，UI 规模可接受，v1e perf）。

**v1d.3 ✅ 完成（main @ f431cb3，家里机 PlayMode 验，修坑 51/52）**：transform 命中渲染 + NativeHost-lite + 整树 dump，检测/矩阵全 core（transform.rs/scene/transform.rs），机制镜像 fgui vertexMatrix（已核实源码）。**transform**（`Affine2=[a,b,c,d,tx,ty]` 列主序 + free fn + Affine2Ext trait；`LocalTransform{matrix:Affine2}` **存矩阵非 TRS 分解**——剪切复合 `scale(2,1) rotate(45deg)` 在解析期 mul 累积不丢；`compute_world_transforms` 每 frame DFS `local=T(rel)∘T(pivot)∘matrix∘T(-pivot)` pivot=box center，`world=parent.world∘local`；`Scene.world_transforms:Vec<Affine2>`）+ **渲染两路径**（identity/merge 走 TRS 零回归；非纯平移 break merge 走 matrix shader `OBJECT_MATRIX` variant 顶点 `mul(_ObjectMatrix,pos)`，剪切天然支持；顶点 re-base 两路径坑 49）+ **命中 world_to_local**（inverse(affine 含剪切可逆)·point → 本地 box (0,0,w,h)）+ **clip 不旋转**（保守留 v1.x）+ **blob v3→v4**（transform 列 local_x/y(2)→world matrix m_a..m_ty(6)，18 列 header_len=84）+ **pkg v5→v6**（ResolvedStyle+transform bincode）+ **NativeHost-lite**（后端纯 C#，div 占位 + BindNativeHost(uint|string)，TRS 分解跟随 world matrix + 显隐 + sort_key→sortingOrder；复用既有 find_node_by_id，core 零改）+ **dump_scene**（FFI 整树 JSON，id/classes 转义）。version v1d.3；1 新常驻 FFI（dump_scene）+ StageHandle+dump_blob；.dll 重编（1709056→1733120B）。spec `docs/superpowers/specs/2026-06-25-v1d.3-transform-nativehost-design.md`、plan `docs/superpowers/plans/2026-06-25-v1d.3-transform-nativehost.md`。**两机约束**：core+ffi cargo test 全绿（core 230+ffi 37+snapshot 3=270）；C# 本机写未编译，家里机 PlayMode（旋转/剪切/缩放容器视觉+子跟随/剪切走 matrix shader/命中 world_to_local/identity 不回归/NativeHost cube 跟随+显隐/DumpScene 日志/500 节点 stress）。subagent-driven 11 task 全 Approved + final review Ready（opus，**跨语言矩阵契约全链核实正确无 Critical**；I1 culling+M1 MPB fix）。踩坑：Affine2 type/mod namespace 冲突（坑 46）、共享 material _ObjectMatrix 覆盖（坑 47）、matrix shader GO identity 致 Mesh.bounds 剔除错位（坑 48）、顶点 re-base 两路径（坑 49）；**家里机验收修 shader matrix design→Unity world 桥接（坑 51）+ Properties 无 Matrix/MPB 必须覆盖 CBUFFER（坑 52）+ stale .dll（坑 10 强化，libloading 加载验证）**。**drag 跟手**（v1d.1 defer）现可落地（transform.translate 已支持）。

**v1d.3 defer（→ v1.x）**：transform-origin 自定义（固定 center）、旋转 clip box / shape mask（clip 不旋转保守）、CSS skew()/matrix() 解析（剪切已由 scale∘rotate 复合支持，skew() 函数本身）、translate % 单位、非均匀缩放∘旋转的 Mesh.bounds 旋转 AABB 扩展（剔除平移已修，旋转留 v1.x）、NativeHost 完整版（layout 测量/size push/hit/clip/所有权/Godot）、动态 UI/panel/实例化（独立子项目）、UI 树可视化 editor（dump 文本已支持）。

**v1d.5 ✅ 完成（main @ e8ef32c，待家里机 PlayMode 验）**：ScrollPane + 滚动条 + 滚轮 + 手势仲裁，**关 v1 验收 #2**，v1d 收尾。机制全 core（scroll.rs 新 952 行），镜像 fgui ScrollPane（已核实源码）。**overflow 扩展**（`overflow_hidden:bool`→`overflow_x/y:OverflowMode` `#[repr(u8)]` Visible/Hidden/Scroll/Auto，Default Visible 零回归；CSS shorthand+longhand；pkg v6→**v7** bincode）+ **offset 虚拟平移注入 world DFS**（`world[N]=world[父]∘T(-父.scroll_pos)∘local[N]`，容器自身不含/后代累积；复用 v1d.3 world matrix + 现有 clip → **blob v4/MirrorPool/shader 零改**）+ **物理自维护 tween 不走 GTween**（drag_follow/inertia/bounce/wheel/set_pos，cubic_out，fgui 常量；**|v| 非 v²** 坑 54；DECELERATION_RATE f64）+ **仲裁 per-slot**（scroll-vs-drag 阈值赛跑先达者赢 + 轴锁 + 嵌套内层优先让出提升；scroll-start cancel click）+ **滚轮**（`WheelEvent` `#[repr(C)]` 16B + `set_wheel_input` FFI + `apply_wheel_to_hit` hit→最近 effective；不产事件）+ **合成 scrollbar**（sentinel node_id `V/H_THUMB_FLAG` + `hit_scrollbar_grip` 前置 + grip 拖拽；坑 55 sentinel 跨链解码）+ **tick 重排**（`compute_world_transforms` 移 process 后，零拖拽延迟；1 帧差 §8.2 认；首帧 guard 防 OOB）。version v1d.5；2 新常驻 FFI（`set_wheel_input`/`set_scroll_pos`）+ 手补 `LoomGUIWheelEvent.cs` 镜像（坑 35）；.dll 重编（1739776→1755136B）。spec `docs/superpowers/specs/2026-06-26-v1d.5-scrollpane-design.md`、plan `docs/superpowers/plans/2026-06-26-v1d.5-scrollpane.md`。**两机约束**：core+ffi cargo test 全绿（core 318+ffi 40+snapshot 3）；C# 本机未编译，家里机 PlayMode 9 点（拖拽跟手/惯性/回弹/滚轮/grip/auto·scroll 显条/嵌套轴锁/scroll-vs-draggable/SetScrollPos+零回归+500 stress）。subagent-driven 12 task 全 Approved + final review Ready（opus，跨任务 seam offset↔仲裁↔tick↔sentinel↔render/hit 逐链 sound，无 Critical/Important）。踩坑：fgui v2 变量名误导 v²→|v|（坑 54）、sentinel id 跨链致 apply_wheel OOB（坑 55）。.meta 留家里机补（坑 13）。

**v1d.5 defer（→ v1.x）**：虚拟化 `<l-list>`、分页/吸附/下拉刷新、滚动条 fade/箭头/点轨道/CSS 定制、shift+滚轮水平、ScrollToView、EVT_SCROLL 事件、滚轮嵌套透传、软裁剪/形状遮罩、padding-edge scroll math（v1 用 border box 简化）。

**v1d 全收尾 + v1 ship-ready**：v1d.1-.5 全完成（§5 全勾）。v1 验收 6 点代码全完成：#1 按钮+文本+图片（v1a/b）/#2 可滚动容器（v1d.5，待家里机验）/#3 hover/active（v1c.1）/#4 safe-area（v1d.1）/#5 is_pointer_on_ui（v1c.1）/#6 打包器二进制包（v1b.1）。**#2 家里机 PlayMode 验收发现 scroll 6 bug 链（坑 58-60：drag 方向反/x 抖/overflow 撑开/子 shrink/sentinel batch 越界/re-base 抵消），已修 332 测绿并提交 2138e52（core: input/scroll/stage/layout/render + dump_scroll）**。**v1-showcase color/img 家里机验收续修**（同批 + 后续 commit）：坑 61 inline style 解析（cascade + `parse_inline_style`）/ 坑 62 Linear 项目 vertex color sRGB→linear（shader 手写 SRGBToLinear）/ 坑 63 font atlas alpha-mask 文字黑（shader `ALPHA_MASK` keyword，MaterialManager `program:1` 启用）/ 坑 64 img UV v 翻转 / 坑 65 img 等比缩放 / letterbox 灰（Main Camera SolidColor #1a1d2e，Driver `ConfigureCameraBackground`）—— 提交 2138e52(core) + 6ce8db3(unity color) + a16f872(showcase 重打 pkg/dll)。**坑 67 layout/render 双测量 text 换行已修·方案 A**（Scene 加 `text_layouts` transient 字段；layout 闭包「Some 优先」存 TextLayout；render 复用不重测，fallback measure 保 test 兼容。推翻早先「浮点边界/epsilon/ceil」症状层猜测——真因是 max_width 来源不一致：layout 用 taffy 选定 known.width，render 用 rect.w。dump_text 验收 72 短标题 before=2→after=1，长文本不变；339 测绿；dump_text.rs 保留为 text 换行诊断 example）。**坑 68 §1.3 img Percent 压扁 + width:auto→0 已修**（Image measure 闭包改消费 taffy `known` 算等比——覆盖坑 65 全 case + Percent；`parse_dimension` 加 auto 分支 + 重打 pkg。dump_img 验收 50%→(500,500) 等比、auto→(64,64) 渲染；340 测绿）。两批（坑 67+68）提交 **25bd50d**。浏览器对照：index.html 加 head 预览（`div{display:flex;flex-direction:column}` + `*{box-sizing:border-box}` 复刻 LoomGUI 契约，scraper select body 不读 head → pkg 不变）。另：清理冗余 demo（db422dc）+ v1e Unchanged 消失修复（7bcc4fd）已提交。

**v1 其余 defer（v0 起，未动）**：
- v1b 全收尾（A/B/C/mesh/CJK ✅）+ v1c.1 最小交互闭环 ✅ + v1c.2 路由完整化 ✅ + v1c.3 多触摸+CaptureTouch ✅ + v1c.4 click 增强 ✅ + v1d.1 拖拽+长按+safe-area ✅（家里机 PlayMode 验，main @ 013b96f，修坑 44/45）+ v1d.2 键盘+焦点+Tab+:focus ✅（家里机 PlayMode 验）+ v1d.3 transform+NativeHost-lite ✅（家里机 PlayMode 验，修坑 51/52）+ v1d.4 GTween tween 引擎 ✅（家里机 PlayMode 验，修坑 53 首帧 dt spike→demo 按钮触发）+ v1d.5 ScrollPane+滚轮+手势仲裁 ✅（main @ e8ef32c，待家里机 PlayMode 验，修坑 54/55；关验收 #2，v1d 全收尾）+ **v1e FFI 同步热路径性能优化 ✅（main @ b52a2a5，Rust 侧完成 + bench 过线；家里机待验 Profiler；Codex §4.3/§6.5 性能债务兑现）**。
- **下一个**：v1 ship（#2 家里机验过即 ship）/ v1.x（Controller/Gear/Transition/TextInput/虚拟化/broadcast/IME）。v1e 范围外推后（撞墙再上）：文本测量 cache / Font Box::leak 缓存化 / invalidation set 伪类重匹配 / Tab 链缓存 / AncestorChain 池化 / FairyBatching 实机 / ReadMesh per-node ArrayPool。.meta 补 commit（坑 13，家里机）。

**v1e（main @ b52a2a5）**：FFI 同步热路径性能债务兑现（Codex §4.3/§6.5）。4 项 → ①dirty hash + Unchanged emit（§2.24，全链路 v0 预留兑现，FFI/blob v4/pkg v7/C# MirrorPool 四零改）静态帧≈0 upload ②冷帧/换页帧 emit ≤2ms（criterion bench：静态 476µs / 冷 1.28ms / 换页 1.19ms 全过线）③C# `_frameBuf` ArrayPool 冷帧零 GC（ReadMesh 留观察）④bench + 家里机 Profiler 双轨验收。subagent-driven 7 task 全 Approved。spec `docs/superpowers/specs/2026-06-26-v1e-perf-design.md`、plan `docs/superpowers/plans/2026-06-26-v1e-perf.md`。家里机待验：PlayMode Profiler 静态帧≈0 upload + 冷/换页帧≤2ms + GC Alloc 静态帧≈0。
- virtualization/shape mask/NativeHost 完整版：v1.x。

完整 defer 表见各 spec §7；v1a Phase 1 实现 ledger 见 `.git/sdd/progress.md`。

**v1-showcase 验收（main 进行中，待家里机 PlayMode 验）**：4 bug + review 最近提交。① bug 1 disabled 按下变蓝（driver 补 `SetNodeDisabled`，CSS `.disabled` 只样式不行为，LoomGUI disabled 是 API 驱动）② bug 2 按下字消失（坑 73 三层修复：_ObjectMatrix 拆 Vector Properties + GetRow + 删 I1 fix）—— **拖动 1.3 img/1.4 span 底消失（scroll 纯平移不走 OBJECT_MATRIX，可能独立 clip/culling）待家里机验**，handoff `docs/v1-showcase-bug2-unity-debug.md`（§3.3 transform 对照实验 + FrameDebugger 步骤）③ bug 3 NativeHost 缩放（坑 72：`_container` 翻正 handedness + per-node wrapper + renderQueue/sortingOrder；非对称模型镜像遗留 v1.x，Cube 对称已够）④ bug 4 §3 蓝底（坑 71 逗号展开）+ §2 Text bg（span→div，Text 节点只画 glyph 跟 fgui GTextField 一致）。review 最近 3 commit（常量全具名对照 fgui；2 中等问题：height:Percent 未 dump 验、UV swap/矩阵下标 `wm[4]` 可读性，记后续）。core test 全绿（343+47）。**不需重编 .dll**（逗号修复 parse-time 进 pkg，pkg 已重打 252520B）；C# + shader 改动 Unity 编。

**大盘点（main @ fe3363b+8649ea3，待家里机验）**：全库清理——core 死代码 8 处（VELOCITY_DECAY_BASE/Font._bytes/version()/LONGPRESS_RADIUS/scroll_down_scroll_pos/new_mouse+new_free→new_slot/tween value_size+_size/layout _html）+ 注释精简 ~430 处（删 v0/v1x/TN/坑N/§N.N 版本标记与开发叙述，留设计契约/算法理由）+ 硬编码命名（SCROLLBAR_TRACK_THICKNESS/MIN_THUMB_SIZE scroll+input 共享/DRAG_FOLLOW_ASSUMED_DT 标注 60fps 假定）+ TextRasterizerTests LoadCjkFont 补全（漏写编译阻断，参照 LoadDejaVu + wqy-microhei.ttc）。core `cargo test --workspace` 全绿（347+47+3）零 warning；.dll 重编 cp（md5 c6352f260dba14e1d8f5c2787170cd2c）。**家里机待办**：① FrameBlob.MeshLen 删（C# private 死代码，列 14 从不读）② LoomEventHandlerTests.BuildStage fontPathBytes 补 `Application.dataPath + "/LoomGUI/Fonts/DejaVuSans.ttf"` 真路径（坑 40 根因占位，9 路由测必红；补 `using UnityEngine`）③ Driver.cs LightLamp `count` 参数 + 6 计数字段清（demo 死状态，可选）④ Unity 整体编译 + EditMode/PlayMode 验注释精简无破坏 + LoadCjkFont 编译通过。

**v1.2 border-radius（main @ 2a467f2，spec `2026-06-29-v1.2-border-radius-design.md`）**：批 1 第 2 项完成。SDD 9 task + 4 fix（spec→plan→9 task→opus final review）。`render/mesh.rs::rounded_rect` 三角扇圆角（照搬 fgui `RoundedRectMesh` 自适应分段/末段精度锁/直角分支 + 两改进：CSS 按边缩放钳制、TL→TR→BR→BL 角序），Container/Button 分支 `%` 渲染期 resolve + all_zero 走 quad 零回归。支持 1~4 值 + `h/v` 椭圆角 + px/%。PKG_FORMAT_VERSION 8→9（坑 74）。showcase §3.7 卡。**final review 抓 I1**（坑 76：dirty hash 采样首末顶点漏圆角 mesh BL-only 变 → >4 顶点 hash 全顶点）。core 385 测全绿。**家里机 PlayMode 验收抓 3 运行时 bug**（本机静态/单测全漏）：坑 77（混合椭圆角直角分支落圆心+方向偏离矩形顶点，9e60302 修）+ 坑 78（scroll+CLIPPED clip rect design/world 空间错位全裁，a34c972 v1 错→e2b64bd v2 修）+ 坑 79（shader tex×vcol 非 CSS 合成，图透明区全透明不透 bg-color，v1.1 spec §6.2 承诺错误）。**方案 A 待实现**（坑 79 修复，`docs/superpowers/plans/2026-06-30-bg-image-composite.md`）：program=2 + `BG_COMPOSITE` keyword 走真合成，blob VERSION 4→5 加 program 列——img 与 Container+bg-image 共用 tex1 须靠 program 分流。

## 维护

每次 LoomGUI 开发/修复后，用 `session-summary` skill 把新踩坑/机制/调试技巧总结进本文件（§5 加坑、§3 加 API、§7 更 ledger）。本 skill 与代码一起提交。
