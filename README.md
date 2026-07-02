# LoomGUI

> 跨引擎游戏 UI 框架。Rust 核心（引擎无关）+ 多引擎后端（Unity 首发），HTML/CSS 子集作 DSL，taffy flexbox 布局，自绘渲染。
>
> **核心目的**：AI 驱动的界面拼装——HTML 作 DSL 让 AI 既能编辑（文本）又能预测渲染结果（AI 对 HTML/CSS 有强先验）。

## 为什么（对标 FairyGUI 的差异化）

- **AI 可预测性**：HTML/CSS-DSL，AI 能读写 + 预测渲染（vs fgui `.fui` 二进制 AI 看不懂）
- **flexbox 布局**：流式 / 响应式 / 内在尺寸（超 fgui 锚点式 Relations）
- **Rust 跨引擎共享核心**：一份核心，多后端（Unity/Godot，vs fgui 各引擎独立 SDK）
- **围栏验证器**：打包期挡违规 CSS，AI 的第一道反馈

## 当前状态

v1 架构走通 + 桌面可演示（Win/Mac Mono）。已交付：渲染/文本/事件/布局/滚动/打包器/FFI/动态树（v1.3+ 代际 NodeId + 命令式 API）/ColorFilter/九宫格/圆角/background-image。

距上线 = v1.x 功能（列表/富文本/Controller/TextInput）+ 编辑器工作流（v other）+ v2 平台（移动/IL2CPP/Godot）。详见 [路线图](docs/roadmap/roadmap.md)。

## 快速上手

```bash
# 核心（引擎无关纯库，可单测）
cargo build -p loomgui_core
cargo test  -p loomgui_core

# 打包器（HTML+CSS+资源 → .pkg.bin + 图集）
cargo build -p loomgui_pkg

# FFI（C ABI，csbindgen 生成 C# 绑定）
cargo build -p loomgui_ffi_c
```

Unity 后端：用 Unity 6.5 打开 `loomgui_unity/`，PlayMode 加载 `.pkg.bin` 渲染。

示例见 `samples/`（v1-showcase 基线 + dyn-mail/leaderboard 动态树 demo + editor 测试场）。

## 文档

- [文档总览](docs/README.md)
- [主设计](docs/design/00-main-design.md) · [围栏权威](docs/design/fence.md) · [路线图](docs/roadmap/roadmap.md)

## 项目结构

| 目录 | 职责 |
|---|---|
| `loomgui_core/` | Rust 核心（解析/样式/布局/场景图/渲染状态/事件/动画/文本，引擎无关纯库） |
| `loomgui_pkg/` | 打包器 CLI（HTML+CSS+资源 → `.pkg.bin` + 图集，复用 core 的 parse 层） |
| `loomgui_ffi_c/` | C ABI 导出（csbindgen，Rust ↔ C# P/Invoke） |
| `loomgui_unity/` | Unity 6.5 URP 后端（GameObject 镜像 + DrawState 缓存 + 输入采集） |
| `editor/` | v other 编辑器工作流模板（open-design 壳 + skill + 围栏规则注入） |
| `samples/` | 示例 + editor 测试场（v1-showcase / dyn-mail / leaderboard / design-systems 夹具） |
| `docs/` | 设计 / 路线 / 文档 |

核心可编译为 WASM（给编辑器）和 C ABI（给引擎），同一份代码。参考实现：FairyGUI-unity（`temp/FairyGUI-unity/`，渲染/对象模型/动画的原理参考）。
