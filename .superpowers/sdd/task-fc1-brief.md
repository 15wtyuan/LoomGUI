## Task 1: 砍 l-container 核心代码 + 围栏契约测试（TDD）

**Files:**
- Modify: `loomgui_core/tests/fence_contract.rs`
- Modify: `loomgui_core/src/parse/dom.rs:29`（FENCE_TAGS）+ 内联测试（dom.rs:203-225）
- Modify: `loomgui_core/src/scene/node.rs:297`

**Interfaces:**
- Consumes: `parse_html(html) -> Result<ElementTree, String>`（parse/dom.rs:32）。
- Produces: FENCE_TAGS 白名单 `["div","span","img","button"]`，`l-container` 成围栏外标签（报错）。

**背景**：l-container 与 div 100% 同映射（node.rs:297），无独特语义，是冗余假自定义元素。砍掉后白名单全 HTML 标准，AI 不困惑 + 预览不塌（Chromium 不认 l-container）。

- [ ] **Step 1: 改 fence_contract 测试（先让它 fail）**

`loomgui_core/tests/fence_contract.rs` 当前 `fence_tags_whitelist_accepted` 测 5 标签含 l-container，`fence_out_tags_rejected` 测 6 标签不含 l-container。改成：白名单去 l-container，被拒加 l-container。

找到 `fence_tags_whitelist_accepted`（约 line 20-26），把 l-container 从白名单循环去掉：
```rust
#[test]
fn fence_tags_whitelist_accepted() {
    // FENCE_TAGS = div/span/img/button（砍 l-container，与 div 同映射冗余）。
    for tag in ["div", "span", "img", "button"] {
        let html = format!("<{tag}></{tag}>");
        assert!(parse_html(&html).is_ok(), "<{tag}> 应被围栏接受");
    }
}
```

找到 `fence_out_tags_rejected`（约 line 29-36），把 l-container 加进被拒列表：
```rust
#[test]
fn fence_out_tags_rejected() {
    // 围栏外标签一律报错，不降级。l-container 砍后是围栏外（用 div）。
    for tag in ["video", "input", "b", "section", "p", "ul", "l-container"] {
        let html = format!("<{tag}></{tag}>");
        assert!(parse_html(&html).is_err(), "<{tag}> 应被围栏拒绝");
    }
}
```

- [ ] **Step 2: 运行测试验证 fail**

Run: `cargo test -p loomgui_core --test fence_contract`
Expected: FAIL——`fence_out_tags_rejected` 的 `l-container` 断言 fail（当前 l-container 还在白名单，parse_html 不报错）。`fence_tags_whitelist_accepted` 应仍 pass（去 l-container 不影响其他 4 个）。

- [ ] **Step 3: 改核心代码——FENCE_TAGS + node.rs 映射 + dom.rs 内联测试**

`loomgui_core/src/parse/dom.rs:29`：
```rust
const FENCE_TAGS: &[&str] = &["div", "span", "img", "button"];
```
（去掉 `"l-container"`）

`loomgui_core/src/scene/node.rs:297`：
```rust
        "div" => NodeKind::Container,
```
（去掉 `| "l-container"`）

`loomgui_core/src/parse/dom.rs` 内联测试（约 line 203-225）：
- `rejects_fence_out_element` 注释 `// 围栏白名单：div/span/img/button/l-container` 改成 `// 围栏白名单：div/span/img/button`。
- `fence_tags_all_accepted`：删掉 l-container 相关断言。当前：
```rust
    fn fence_tags_all_accepted() {
        // 白名单内五种 tag 均通过（l-container 同 div）
        let html = r#"<div><span>x</span><img src="a.png"><button>ok</button></div>"#;
        let tree = parse_html(html).unwrap();
        assert_eq!(tree.roots.len(), 1);
        let lcontainer = parse_html(r#"<l-container></l-container>"#).unwrap();
        assert_eq!(lcontainer.nodes[lcontainer.roots[0].0].tag, "l-container");
    }
```
改成（删 l-container 断言 + 注释）：
```rust
    fn fence_tags_all_accepted() {
        // 白名单内四种 tag 均通过（l-container 砍，与 div 同映射冗余）
        let html = r#"<div><span>x</span><img src="a.png"><button>ok</button></div>"#;
        let tree = parse_html(html).unwrap();
        assert_eq!(tree.roots.len(), 1);
    }
```

- [ ] **Step 4: 运行测试验证 pass**

Run: `cargo test -p loomgui_core --test fence_contract`
Expected: 10 passed（含改后的 whitelist 4 标签 + l-container 被拒）。

再跑 dom.rs 内联测试：
Run: `cargo test -p loomgui_core --lib parse::dom`
Expected: pass（fence_tags_all_accepted + rejects_fence_out_element 都过）。

- [ ] **Step 5: commit**

```bash
git add loomgui_core/src/parse/dom.rs loomgui_core/src/scene/node.rs loomgui_core/tests/fence_contract.rs
git commit -m "feat(core): 砍 l-container 出围栏白名单（与 div 同映射冗余）"
```

---

## Task 2: 围栏文档同步去 l-container（fence.md + editor 副本 + rules 模板）
