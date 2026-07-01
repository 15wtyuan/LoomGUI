## Task 2: 围栏文档同步去 l-container（fence.md + editor 副本 + rules 模板）

**Files:**
- Modify: `docs/design/fence.md`（§1 元素白名单表）
- Modify: `editor/skill/loomgui-editor/references/fence.md`（同步副本）
- Modify: `editor/rules/claude/CLAUDE.md.tmpl` + `editor/rules/opencode/AGENTS.md.tmpl` + `editor/rules/codex/AGENTS.md.tmpl`（元素白名单段）

**Interfaces:**
- Consumes: Task 1 的 FENCE_TAGS 新白名单 `["div","span","img","button"]`。
- Produces: 围栏文档副本与权威一致（坑 83 防漂移）。

- [ ] **Step 1: 改 docs/design/fence.md §1 元素白名单表**

`docs/design/fence.md` §1 有元素白名单表（约 line 27-34），含 l-container 行。删掉 l-container 行：
```markdown
| 标签 | 映射 NodeKind | 出处 |
|---|---|---|
| `div` | Container | scene/node.rs:278 |
| `span` | Text（内容取 `el.text`） | scene/node.rs:283 |
| `img` | Image（src 取 `el.attrs["src"]`） | scene/node.rs:280 |
| `button` | Button | scene/node.rs:279 |
| `l-container` | Container（与 div 同） | scene/node.rs:278 |
```
改成（删 l-container 行）：
```markdown
| 标签 | 映射 NodeKind | 出处 |
|---|---|---|
| `div` | Container | scene/node.rs:278 |
| `span` | Text（内容取 `el.text`） | scene/node.rs:283 |
| `img` | Image（src 取 `el.attrs["src"]`） | scene/node.rs:280 |
| `button` | Button | scene/node.rs:279 |
```

§1 还有"白名单（`FENCE_TAGS`，parse/dom.rs:29）"后面的描述若提 l-container 也删。搜 `l-container` 全文，逐处删（§0 反例段不提 l-container，不用改）。

- [ ] **Step 2: 同步 editor 副本（坑 83 防漂移）**

```bash
cp docs/design/fence.md editor/skill/loomgui-editor/references/fence.md
diff -q docs/design/fence.md editor/skill/loomgui-editor/references/fence.md
```
Expected: `diff -q` 无输出（byte-identical）。

- [ ] **Step 3: 改三个 rules 模板的元素白名单段**

`editor/rules/claude/CLAUDE.md.tmpl` 元素白名单段当前：
```markdown
## 元素白名单
只用 `div` / `span`（+裸文本）/ `img` / `button` / `l-container`。其他标签（video/input/p/ul/...）会报错。
```
改成：
```markdown
## 元素白名单
只用 `div` / `span`（+裸文本）/ `img` / `button`。其他标签（video/input/p/ul/...）会报错。
```

opencode/codex 的 AGENTS.md.tmpl 内容与 claude/CLAUDE.md.tmpl 完全相同，同样改。改完三个文件用 diff 验证一致：
```bash
diff editor/rules/claude/CLAUDE.md.tmpl editor/rules/opencode/AGENTS.md.tmpl
diff editor/rules/claude/CLAUDE.md.tmpl editor/rules/codex/AGENTS.md.tmpl
```
Expected: 均无输出（三模板 byte-identical）。

- [ ] **Step 4: commit**

```bash
git add docs/design/fence.md editor/skill/loomgui-editor/references/fence.md editor/rules/
git commit -m "docs(fence): 围栏文档同步去 l-container（fence.md + editor 副本 + rules 模板）"
```

---

## Task 3: 设计文档同步去 l-container（00-main-design + v1-scope）
