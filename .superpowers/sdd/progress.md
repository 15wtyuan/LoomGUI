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
