# v1.4-a SDD Progress Ledger

Plan: docs/superpowers/plans/2026-07-02-v1.4a-package-loading.md
Spec: docs/superpowers/specs/2026-07-02-v1.4a-package-loading-design.md
Worktree: .claude/worktrees/v1.4a-package-loading (branch worktree-v1.4a-package-loading)
BASE=7a01d77

## Tasks
- T1: complete (commits 7a01d77..07fccdd, review clean after fix) — pkg 多组件格式 + read/write + 防御缺口修复
- T2: complete (commits 07fccdd..cf3ed34, review clean) — path/CSS 归一化（scraper 抽 CSS 绕围栏，T3 须先抽 CSS 再 parse_html）
- T3: complete (commits cf3ed34..0a0d855, review clean) — 打包器多 HTML + 砍 image/atlas + CLI
- T4: complete (commit pending) — Stage 资源池 + load_package 不建 scene + 砍 textures/atlases/load_inline/build_registry
- T4: complete (commits 0a0d855..fa19bc0, review clean) — load_package 进资源池不建 scene + 砍 textures/atlases/load_inline. texture.rs 留 T6 删；3 测试 ignore（package_load_renders_identical_to_inline/hover/disabled）T5 必须重写恢复
- T5: complete (commit pending) — Stage::instantiate 克隆组件子树 + 伪类规则合并去重 + create_node_from_template 复用节点构造 + ensure_scene 首次建空骨架. 3 个 T4 ignore 测试重写恢复（load_package→instantiate→roots.push→render 等价 inline）. 492 core 测试全过, ignored 9→6（FFI 6 留 T7）
- T5: complete (commits fa19bc0..3ac7a92, review clean after fix) — instantiate 克隆子树 + 伪类去重 + 3 T4 测试债还清 + 多实例 hover 独立性测试
- T6: complete (commit pending) — Image RenderNode payload 改带 image_path (砍 texture/UV子区/tex_id/fit_uv/TextureRegistry/TexMeta/asset::texture.rs). layout solve 砍 textures 参数 (Image intrinsic 64×64 兜底). FrameBlob v7: tex_id 列→path_idx 列 + path string table arena. 556 workspace 测试全过 (486 core + 45 ffi + 余). FFI blob 同步改完 (非 T7 stub).
- T6: complete (commits 3ac7a92..c4d488e, review clean after D17 fix) — Image payload path + 砍 TextureRegistry + D17 图尺寸表(打包期 PNG IHDR). spec 补 D17
- T7: complete (commits c4d488e..262e0fb, review clean) — FFI load_package name + instantiate + 砍 atlas FFI + csbindgen regen + .dll. Unity LoomStage.cs 断裂 T8 修
- T8: complete (commits 262e0fb..7223b95, review clean) — Unity path→Sprite + Sprite Atlas + 砍 LoadAtlas/_texMap. 旧 v4 测试/showcase driver T11/T12 还债
- T9: complete (commit pending, 家里机编译验证) — Unity 包管理面板：LoomPackageSettings ScriptableObject + LoomPackageManagerWindow EditorWindow（智能识别/刷新/一键打包/资源校验）+ PkgManifestReader(C# 解析 pkg.bin AssetManifest). 本机无 Unity 工具链，C# 语法核对+grep 验证，家里机编译验证
- T9: complete (commits 7223b95..43b7aed, review clean) — Unity 包管理面板 + PkgManifestReader(C# 手解析 pkg.bin manifest)
- T10: complete (commit pending) — showcase 拆多组件包：home + 7 page + tips_toast + mail + res/icons(15 PNG). 打包 10 components/4 manifest paths, roundtrip 验证 OK (根 parent_idx=None, D17 尺寸 64x64/108x108). img path 契约: HTML 须带 res/ 前缀. 旧 atlas.png 清理 (D8/D14). 旧 samples 归档保留
- T10: complete (commits 43b7aed..6bb9998, review clean) — showcase 拆 10 组件包, 打包成功. T11 接 nav→instantiate + dyn-load-mail→同包 instantiate
- T11: complete (commits 6bb9998..5cfde70, review clean after fix) — driver 重写 layer+nav+按页listener清+tips叠加. NativeHost 跨页摘修. tips/mail 叠加非覆盖(v1 无 absolute) v1.4-b 补
