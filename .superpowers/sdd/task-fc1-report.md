# Task FC1 Report: 砍 l-container 核心代码 + 围栏契约测试（TDD）

## 改的文件

| 文件 | 变更 |
|---|---|
| `loomgui_core/tests/fence_contract.rs` | `fence_tags_whitelist_accepted` 去 `"l-container"`；`fence_out_tags_rejected` 加 `"l-container"` |
| `loomgui_core/src/parse/dom.rs` | FENCE_TAGS 去 `"l-container"`；内联测试 `fence_tags_all_accepted` 删 l-container 断言 |
| `loomgui_core/src/scene/node.rs` | `"div" \| "l-container"` → `"div"`（砍 l-container 映射） |

## Step 2: fail 输出

```
running 10 tests
... 9 passed ...
test fence_out_tags_rejected ... FAILED

---- fence_out_tags_rejected stdout ----
thread 'fence_out_tags_rejected' (45344) panicked at loomgui_core\tests\fence_contract.rs:30:9:
<l-container> 应被围栏拒绝

test result: FAILED. 9 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

`fence_out_tags_rejected` 的 `l-container` 断言 fail —— l-container 尚在白名单，`parse_html` 不报错。`fence_tags_whitelist_accepted` 仍 pass（4 标签无影响）。符合预期。

## Step 4: pass 输出

### `cargo test -p loomgui_core --test fence_contract`

```
running 10 tests
... all ok ...
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### `cargo test -p loomgui_core --lib parse::dom`

```
running 8 tests
test parse::dom::tests::rejects_fence_out_element ... ok
test parse::dom::tests::fence_tags_all_accepted ... ok
... all 8 ok ...
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 409 filtered out
```

## Commit

- **hash**: `67b383e99f207a0fc73eac723abca07c54949fea`
- **branch**: `worktree-fence-cleanup`
- **message**: `feat(core): 砍 l-container 出围栏白名单（与 div 同映射冗余）`

## Concerns

- 无。改动最小且精准：3 个文件，+8/-10 行。

## Fix round 1

- **文件**: `loomgui_core/src/scene/node.rs` line 273
- **改动**: 注释 `div/span/img/button/l-container` → `div/span/img/button`（砍 l-container）
- **commit**: `990d3e6bd9ffd2db55ae44c2222ed8017c1f906b`
