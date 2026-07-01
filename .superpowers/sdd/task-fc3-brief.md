## Task 3: 设计文档同步去 l-container（00-main-design + v1-scope）

**Files:**
- Modify: `docs/design/00-main-design.md:122`（元素表）
- Modify: `docs/roadmap/v1-scope.md` §2（元素行）

**Interfaces:**
- Consumes: Task 1 新白名单。

- [ ] **Step 1: 改 00-main-design.md 元素表**

`docs/design/00-main-design.md:122` 当前：
```markdown
| `<div>` / `<l-container>` | Container | 通用 flex 容器，可裁剪/遮罩，可挂 ScrollPane |
```
改成：
```markdown
| `<div>` | Container | 通用 flex 容器，可裁剪/遮罩，可挂 ScrollPane |
```

搜 00-main-design.md 全文 `l-container`，若有其他提及（如示例代码）也删/改。`l-` 前缀原则那句（:111"自定义元素 kebab-case：`<l-list>`/`<l-loader>` 等用 `l-` 前缀"）**保留**——l-list/l-rich 等 v1.x 真自定义元素仍用 l- 前缀，只砍 l-container。

- [ ] **Step 2: 改 v1-scope.md §2 元素行**

`docs/roadmap/v1-scope.md` §2 元素行当前（约 line 55）：
```markdown
**元素**：`div`(Container) / `span`+裸文本(Text) / `img`(Image) / `button`(Button) / `l-container`(Container，与 div 同)。
```
改成：
```markdown
**元素**：`div`(Container) / `span`+裸文本(Text) / `img`(Image) / `button`(Button)。
```

§2 上方"纠正"段（约 line 53）若提 l-container 也删：
```markdown
> **纠正**（fence.md 核实）：`position:relative` 靠 taffy 默认 Relative 生效（非显式映射，写不写一致）；`font-style` 无 handler 静默忽略（原 §2 误列支持）；`l-container` 与 div 同映射（原 §2 漏列）。
```
改成（删 l-container 那句，因已砍）：
```markdown
> **纠正**（fence.md 核实）：`position:relative` 靠 taffy 默认 Relative 生效（非显式映射，写不写一致）；`font-style` 无 handler 静默忽略（原 §2 误列支持）。
```

- [ ] **Step 3: commit**

```bash
git add docs/design/00-main-design.md docs/roadmap/v1-scope.md
git commit -m "docs(design): 00-main-design + v1-scope 去 l-container"
```

---

## Task 4: 严 polyfill 固化进 skill
