# v1b.5 — CJK 文本（unicode-linebreak 逐字断行 + CJK 字体）设计

> 日期 2026-06-21 | 分支 `feat/v1b.5-cjk-text`（base main @ `8e565e1`）
> 关联：design §9（文本）/ v1x-deferred §5（文本 defer）/ knowledge §2.4
> 范围：v1b 拆分 **D（文本 CJK）的最小可行子集** —— A 档「最小 CJK」

---

## 1. 背景与动机

- v1b 拆分 A 打包器（✅ v1b.1）/ B 真纹理（✅ v1b.2）/ C 图集（✅ v1b.3）/ mesh 合并（✅ v1b.4）/ **D 文本 CJK**。D 是 v1b 最后一项。
- D 原描述「文本 CJK/多字体（text_arena 升三表、font fallback）」。
- 本次缩到 **最小 CJK（A 档）**：兑现 design §9.1「v1 仅支持 CJK + ASCII + CJK 标点」承诺。
- 首发亚洲/国内市场 → 中文正确渲染 + 逐字换行是 v1 硬需求。
- design line 608 明确「runs/lines 三表随多字体/CJK 于 v1b 落地」——但 A 档只取「CJK 落地」，多字体/三表升级 defer（单字体下零收益）。

---

## 2. 现状（text/layout.rs 的 4 个待解点）

1. `Font` = 单 ttf `Face<'static>`（`Box::leak`，单字体无 fallback，进程级泄漏）。
2. `measure_text(font: &Font)` 单字体；缺字 `glyph_index(ch).unwrap_or_default()` → `.notdef`（tofu），无 fallback。
3. **断行是 `content.split(' ')` 贪心（layout.rs:196）——对 CJK 完全失效**（中文无空格 → 整段当一个 word 无法换行）。← **D 要修的核心 bug**。代码注释自承「unicode-linebreak 留作 v1.x」。
4. `TextLayout` 三表（lines/runs/glyphs）Rust 侧已有；但 blob `text_arena` 是扁平 glyph 表（per-node `glyphs[{codepoint,pen_x,pen_y}]`）。

---

## 3. 范围（A 档 = 最小 CJK）

**做（3 件事）**：
1. **断行**：引 `unicode-linebreak` crate，`measure_text` 断行 `split(' ')` 贪心 → UAX#14（CJK 逐字可断、ASCII 按词边界）。
2. **字体**：Stage/Unity 接入一张 CJK ttf（文泉驿微米黑），让中文能度量（Rust）+ 光栅（Unity）。
3. **验证**：fixture 加 CJK 字体 + Rust 断行测 + Unity CJK 光栅 + sample 加中文段落。

**不做（defer / YAGNI 清单）**：
- **font fallback 链**（主字体缺字切备用）—— design §9.1「fallback 链 v1 砍」；缺字返 `.notdef` tofu。
- **多 font-family / per-run per-glyph font_id / TextLayout runs 三表投 blob** —— A 档单字体下零收益。
- **emoji / 组合符号 shaping / RTL** —— v1x-deferred §5。
- **kinsoku 行首行尾标点禁则** —— §9.1「实现期对照 Chrome 调，不钉死算法」，defer。
- **measure 缓存** —— v1x-deferred §5「撞墙再加」。
- **line-height/baseline 校准** —— 现状占位（layout.rs:143-148）不动。
- **Font `Box::leak` 缓存化** —— v1e perf（CJK ~5MB leak 更显眼但非阻塞）。

---

## 4. 硬不变量（blob / 后端零改契约）

- **blob v3 / MirrorPool / TextRasterizer / shader 零改**：
  - CJK codepoint ≤ U+FFFF（BMP），现有 `codepoint:u32` 容得下，Unity `GetCharacterInfo((char)codepoint)` 正常（UTF-16 BMP 内 1:1）。emoji > U+FFFF 才需 surrogate，但 A 档 defer emoji。
  - A 档只动 `measure_text` 断行逻辑 + 字体文件，blob 结构/读写路径不动。
- **单字体模型不变**：`Stage::new(font_path)` 签名不变、`measure_text(font: &Font)` 签名不变。
- **CSS `font-family` 记录但不消费** —— A 档忽略 font-family 值差异（维持现状，measure 用 Stage 单字体）。
- 验证手段：`git diff --stat` 确认 blob.rs / FrameBlob.cs / MirrorPool.cs / TextRasterizer.cs / shader 零生产 diff（仅测文件可加 CJK case；LoomStage.cs 接线小改见 §7）。

---

## 5. measure_text 断行改造

**改动范围**：仅 `layout.rs:194-220` 断行块（`split(' ')` 贪心 → unicode-linebreak + greedy fill）。`measure_width` / `kerning` / `advance` / glyph 生成 / `Font` 全不动。

**新依赖**：`unicode-linebreak`（pure Rust、无传递依赖、轻量），加 `loomgui_core/Cargo.toml`。**版本实现期验** crates.io 实际（坑 1/2/8 教训：brief/草稿的 crate API 常不符）。

**算法**（greedy fill on break opportunities，结构对齐现状贪心，只换「切词」方式）：
1. `linebreaks(content)` → 所有换行机会 `Vec<(byte_offset, BreakType{Mandatory|Allowed})>`。
2. 按 offset 切 segments —— unicode-linebreak 在空白**后**断，segment 含尾空白 → 行首无多余空格（现状 `split(' ')` 丢空格重加 `space_w` 的逻辑整体删除，segment 宽度自含空白）。
3. 贪心填行：累加 segment 宽，超 `max_width` 在最近的 Allowed break 换行；`Mandatory`（`\n`）强制结束当前行。
4. `nowrap=true` → 忽略所有 break（含 `\n`），强制单行（对齐 CSS `white-space:nowrap`）。
5. `max_width=None` → 只 Mandatory break 生效（无约束不软换行）。

**CJK 行为**：unicode-linebreak 对 CJK 字符间给 `Allowed` → 逐字可断（兑现 §9.1「CJK 逐字」）。ASCII 按词边界，与现状 `split(' ')` 结果等价（按词换行）。

**边界 case**：
- **超长词**（无 break point 的长串，如长 URL / 连续 CJK+数字混排）：unicode-linebreak 不给中间 break → 可能溢出。**参考 fgui**（§6）`wordLen≥20 或非词 → toMoveChars=1`（强制逐字断）语义；实现期加此边界，阈值参考 fgui 的 20。
- **缺字**：`glyph_index(ch)` 缺字返 `.notdef`（tofu），维持现状，A 档无 fallback。
- **`\n`**：Mandatory 强制换行（现状 `split(' ')` 把 `\n` 当普通字符，A 档改进为强制换行）。

**回归风险**：`v0_snapshot` / `text::layout::tests`（ASCII）。跑 snapshot 确认 ASCII 断行不漂移；若 unicode-linebreak 与 `split(' ')` 在 ASCII 边界有字节级差异致 snapshot 变，更新 snapshot（语义不变坏，review 确认）。

---

## 6. fgui 对照（BuildLines，TextField.cs:760-934）

fgui **没有** 用 unicode-linebreak，用更简的字符集合启发式：
- 逐 char + `wordPossible/wordLen` 状态机。
- **词字符集** `[a-zA-Z0-9."']`（line 821-823）。CJK / 标点 / 其它**不在**此集 → `wordPossible=false`。
- **超宽换行**（line 881-933）：`wordPossible && wordLen<20` → 整词移下行（英文词不拆，`<20` 防超长词/URL 撑爆行）；**else → `toMoveChars=1`（CJK 逐字断）**。
- `\n` 强制换行；`_singleLine`（=nowrap）不换。

**目标行为与 unicode-linebreak 一致**（CJK 逐字 + 英文词不拆 + `\n` + nowrap），fgui 用零依赖更简实现，经多项目验证。

**取舍**：本次选 **unicode-linebreak**（对齐 Chrome → AI 可预测性 = LoomGUI 首要准则；design §9.1 契约兑现）。fgui 的 `wordLen<20` + `toMoveChars=1` 作为**超长词边界参考**（§5）。若将来想零依赖最简，可改 fgui 式（需同步改 design §9.1 契约）。

---

## 7. 字体加载 + fixture + Unity 接线

**原则：纯增量，最小侵入** —— 保留 DejaVu 为默认 ASCII 字体（现有 Stage/测/sample/snapshot 全不动，零回归），**新增** CJK 字体作可选第二字体，CJK 验证用一个吃 CJK ttc 的 Stage。

**Rust fixture**：
- 新增 `loomgui_core/tests/fixtures/wqy-microhei.ttc`（文泉驿微米黑，~5MB，GPL 开源）。
- 新增 `test_font_cjk()` helper（skip-if-missing，沿用 `test_font()` 模式）。
- `loomgui_ffi_c` 测 `font_path()` 不动（仍 DejaVu，FFI 结构测不需 CJK）。

**Stage / Font**：
- `Stage::new(font_path)` 签名不变（本就接受任意路径，传 CJK ttc 路径即得 CJK Stage）。
- `Face::parse(bytes, 0)` 对 `.ttc` 取 index 0 face（文泉驿微米黑 Regular）。`.ttc` vs `.ttf` 实现期验（若 Unity FontImporter 对 .ttc 有问题，备选 .ttf 单文件版）。

**Unity 接线（LoomStage.cs 小改）**：
- 加 `[SerializeField] string _fontFile = "DejaVuSans.ttf"`（可配字体文件名，**默认值不变 → 现有场景零改**）。
- `LoomStage.cs:75` fontPath 用 `_fontFile` 拼（不再硬编码 `"DejaVuSans.ttf"`）。
- 新增 `Assets/LoomGUI/Fonts/wqy-microhei.ttc` + `StreamingAssets/wqy-microhei.ttc`（CJK 字体两份，§4.3 Rust/Unity 同 ttf 一致性）。

**CJK sample**：
- `loomgui_pkg/samples/cjk/{page.html,page.css}`：一个 div，窄宽（~200px）+ 中文段落（含 CJK + ASCII 混排 + CJK 标点），触发逐字换行 + 混排。
- 打包 → `.pkg.bin` → LoomStage（`_fontFile = wqy-microhei.ttc`，Inspector `_font` 指 CJK Font）。package 路径（对齐 v1b.3/.4 sample 模式）。

---

## 8. 测试策略

**Rust（`loomgui_core`，CJK 字体 skip-if-missing）**：
- `text::layout::tests` 新增：
  - CJK 逐字断行（窄约束下中文 ≥2 行）。
  - CJK + ASCII 混排断行。
  - `\n` mandatory break 强制换行。
  - `nowrap` 不换（含 CJK 长文本）。
  - 超长词边界（>阈值无空格串逐字断，参考 fgui）。
- 现有 4 测（`single_line_ascii` / `wraps_on_width` / `nowrap_never_wraps` / `line_height_scales`，用 DejaVu）不破。
- `v0_snapshot`：若含断行文本，unicode-linebreak 改断行可能漂移 → 跑确认/更新。

**Unity（`loomgui_unity`）**：
- EditMode：`TextRasterizerTests` 加 CJK case（沿用「编译 + 逻辑正确，预期值内部重算」模式，headless 不锁 DejaVu/CJK 具体数值）。
- PlayMode（**验收，押用户**）：CJK sample → 中文正确渲染 + 逐字断行 + 无 tofu。

---

## 9. 验收标准（对齐 v1b.4 模式）

- ✅ Rust：CJK 断行测过 + ASCII 回归不破 + `cargo test --workspace` 绿。
- ✅ blob.rs / FrameBlob.cs / MirrorPool.cs / TextRasterizer.cs / shader 零改（`git diff --stat` 确认这些文件零 diff；LoomStage.cs 仅加 `_fontFile` 字段 + fontPath 用它拼，见 §7）。
- ✅ Unity EditMode：CJK 光栅逻辑测编译过。
- ✅ PlayMode：CJK sample 中文正确渲染 + 逐字换行 + 无 tofu（**用户验收**）。
- ✅ design §9.1「CJK + ASCII + CJK 标点」v1 承诺兑现。

---

## 10. .dll 重编

core 引 `unicode-linebreak` 改断行 → `cargo build --release` → **关 Unity**（锁 .dll）→ `cp target/release/loomgui_ffi_c.dll loomgui_unity/Assets/Plugins/LoomGUI/`（坑 10）。blob 格式零改，重编仅为断行逻辑 + 新依赖。

---

## 11. 风险

- **unicode-linebreak API 与草稿不符**（坑 1/2/8）→ 实现期验 `~/.cargo/registry/src/<unicode-linebreak>-<ver>/src/` 实际签名（`linebreaks` 返回类型、`BreakType` 变体名、offset 语义「断在前/后」）。
- **ASCII snapshot 漂移**（unicode-linebreak vs `split(' ')`）→ 跑确认，语义不变坏则更新 snapshot。
- **`.ttc` Unity import**（FontImporter 对 collection 支持）→ 实现期验，`.ttf` 单文件版备选。
- **CJK 字体 ~5MB 入仓** → 用户已确认「现成轻量 CJK 字体」策略（接受体积换自包含 + CI 覆盖）。
- **Font `Box::leak` × CJK 5MB** → 每 Stage leak 一份 5MB，域重载累积。v1e 缓存化根治；A 档非阻塞（×20 域重载观察，<阈值推 v1e，对齐 v1a Phase 2 Font leak 处理）。
