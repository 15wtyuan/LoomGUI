# FC Task 3 Report: 设计文档同步去 l-container

## 改动汇总

| 文件 | 行 | 操作 |
|---|---|---|
| `docs/design/00-main-design.md` | 122 | `<div>` / `<l-container>` → `<div>` |
| `docs/roadmap/v1-scope.md` | 53 | 纠正段删 `；`l-container` 与 div 同映射（原 §2 漏列）` |
| `docs/roadmap/v1-scope.md` | 55 | 元素行删 ` / `l-container`(Container，与 div 同)` |

## l- 前缀原则保留确认

`docs/design/00-main-design.md:111` 原文未改动：
> `- **自定义元素 kebab-case**：`<l-list>`/`<l-loader>` 等用 `l-` 前缀避免与 HTML 冲突。`

两文件全文 grep `l-container` 均为 0 命中。

## Commit

```
9b69264 docs(design): 00-main-design + v1-scope 去 l-container
```

## Concerns

无。最小改动，原则保留，grep 干净。
