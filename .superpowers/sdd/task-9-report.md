# Task 9 报告：Unity 包管理编辑器面板

**状态**：已实现（本机无 Unity 工具链，C# 语法核对 + grep 验证；家里机编译验证）
**Commit**：（待填）

## 实现

### 文件（新建，全部在 `loomgui_unity/Assets/LoomGUI/Editor/`）

1. **`LoomPackageSettings.cs`** — ScriptableObject 配置资产。
   - `LoomPackageSettings : ScriptableObject`：`resDirName`（默认 "res"）、`pkgOutputDir`（默认 "Assets/StreamingAssets/"）、`loomPkgExePath`（默认 "../target/release/loomgui_pkg.exe"）、`List<PackageEntry> packages`。
   - `[CreateAssetMenu(menuName = "LoomGUI/Package Settings")]` 供菜单创建。
   - `GetOrCreateDefault()` 静态方法：面板首次打开若无配置资产自动建到 `Assets/LoomGUI/Editor/LoomPackageSettings.asset`。
   - `PackageEntry`：`pkgName` + `sourceDir`（Unity 工程相对路径）+ `List<string> htmlFiles`。

2. **`LoomPackageManagerWindow.cs`** — EditorWindow 主面板（`[MenuItem("LoomGUI/Package Manager")]`）。
   - **智能识别**：拖目录到顶部 DropZone → `pkgName=目录名` + 扫顶层 `*.html`（`SearchOption.TopDirectoryOnly`，排除 `res` 目录同名文件）→ 建 `PackageEntry`。sourceDir 转 Unity 工程相对路径存。
   - **包列表编辑**：每包独立卡片，pkgName/sourceDir 可编辑，htmlFiles 可增删，嵌套子目录的 .html 可单独拖入补漏（拖区在每包卡片底部）。
   - **刷新按钮**（每包）：重扫 sourceDir 顶层 .html，diff htmlFiles——新增加入、删除移除（用"无路径分隔符 + 不在新扫描集"判删除，区分顶层扫描文件 vs 手动加的子目录 html）、保留手动加的非顶层 html + 改过的 pkgName。
   - **全局配置**：resDirName / pkgOutputDir / loomPkgExePath 三栏，改动走 `Undo.RecordObject` + `EditorUtility.SetDirty`。
   - **一键打包**（每包 + 全部）：起 `loomgui_pkg.exe` 子进程，args = `"<sourceDir>" <pkgName> --html <h1,h2,...> --res <name> -o "<out>"`，redirect stdout/stderr，显示日志（exit code + rich text 红绿标记成败），打包后 `AssetDatabase.Refresh()`。
   - **资源校验**（每包 + 全部）：读 pkg.bin 的 AssetManifest（调 `PkgManifestReader`），对每个 manifest path 校验 `sourceDir/res/<path>` 文件存在，缺失红字列出 + 汇总"缺 N/M 条"。

3. **`PkgManifestReader.cs`** — C# 读 pkg.bin 末尾 AssetManifest 的轻量解析器（不引 bincode）。
   - 解析路径：Header(20B) → StringTable → ComponentTable → 逐节点跳过 NodeBlock（按每节点 `style_len` + class_count 跳）→ 跳 PerComponentDynamicRules（按 ComponentTable dynamic_len 之和跳）→ 读 AssetManifest（`entry_count + count×{path_idx, w, h}`），path_idx 查 StringTable 还原 path 字符串。
   - 校验 magic (`0x474B504C`) + version (12)，错抛 `PkgManifestException`（面板显示红字）。
   - 越界 / 截断防御：所有读都校验长度，越界抛异常不崩。

4. **`LoomGUI.Editor.asmdef`** — Editor 程序集定义。`includePlatforms: ["Editor"]`、`references: ["LoomGUI.Runtime"]`、`autoReferenced: true`。

### 关键决策

**AssetManifest 读取（spec §6.2-4）**：选择了"C# 手解析 pkg.bin 末尾 manifest 段"而非"调 Rust 工具读"或"只扫 res/ 不读 pkg.bin"。

- 为何不调 Rust 工具：多一个子进程依赖 + 需要一个专门的 manifest-dump CLI（T3 的 loomgui_pkg 只打包不 dump manifest），成本高。
- 为何不只扫 res/ 磁盘：spec §6.2-4 明确"读 pkg.bin AssetManifest 校验 res 齐全"——manifest 是打包器归一化后的真实 path 列表（去重、去 res 前缀、正斜杠归一），直接扫 res/ 无法对应到 pkg.bin 里的 path（路径分隔符/前缀差异）。读 manifest 才能精确校验"pkg 引用的图都有对应文件"。
- 解析复杂度：~140 行，只读 manifest 段不碰 bincode（ResolvedStyle/DynamicRuleTable 不解析），用 Header 里的 string_count/component_count + ComponentTable 里的 dynamic_len 跳过中间段。可行、最小可用。

**loomgui_pkg.exe 定位**：配置在 `LoomPackageSettings.loomPkgExePath`（默认 "../target/release/loomgui_pkg.exe"，工程根相对）。面板解析时若工程根相对则拼 `Application.dataPath/..` + 配置值；Windows 自动补 .exe 后缀。找不到 exe 时红字提示"请先 cargo build --release -p loomgui_pkg"。这样兼容本机/家里机不同 build 输出位置（用户改配置即可）。

**路径处理**：sourceDir / pkgOutputDir 统一存 Unity 工程根相对路径（跨机器可移植）。打包时转绝对路径传 CLI；htmlFiles 存相对 sourceDir 的文件名（顶层）或相对路径（嵌套子目录补拖）。

## 自审

### Unity Editor API 核对（grep 验证通过）
- `EditorWindow` / `GetWindow<T>(focus, title, utility)` ✓
- `[MenuItem("LoomGUI/Package Manager")]` ✓
- `DragAndDrop.AcceptDrag()` / `DragAndDrop.paths` / `DragAndDrop.visualMode` / `DragAndDropVisualMode.Copy/Rejected` ✓
- `Event.current.type == EventType.DragPerform/DragUpdated` ✓
- `GUILayoutUtility.GetRect(width, height, GUILayoutOption[])` ✓
- `EditorGUILayout.TextField/LabelField/Space/BeginScrollView/EndScrollView/BeginHorizontal/EndHorizontal/BeginVertical/EndVertical` ✓
- `EditorGUI.BeginChangeCheck/EndChangeCheck` ✓
- `Undo.RecordObject` ✓
- `AssetDatabase.LoadAssetAtPath/CreateAsset/SaveAssets/Refresh` ✓
- `EditorUtility.SetDirty` ✓
- `EditorStyles.boldLabel/helpBox/label/miniLabel/wordWrappedMiniLabel` ✓（均为有效字段）
- `GUI.Box` / `GUILayout.Button` ✓
- `GUIUtility.ExitGUI()` ✓（删包/删 htmlFile 后重绘，已匹配 EndHorizontal 再 Exit 避免 layout 栈失衡）
- `Selection.activeObject` ✓
- `Process.Start(ProcessStartInfo)` + `RedirectStandardOutput/Error` + `UseShellExecute=false` ✓
- `Application.dataPath` / `Application.platform` / `RuntimePlatform.WindowsEditor` ✓

### C# 语法核对
- 括号配平：三文件 open/close 全等（grep 计数验证）。
- `using` 齐全：`UnityEditor`（AssetDatabase）、`UnityEngine`（ScriptableObject/Application）、`System.Diagnostics`（Process）、`System.IO`（File/Directory/Path）、`System.Text`（StringBuilder/Encoding）。
- `Skip` 重载歧义消解：常量表达式显式 `(uint)` cast（`Skip((uint)(2+2))`）。
- 删包/删 htmlFile 的 `ExitGUI` 前先 `EndHorizontal`，layout 栈平衡（`try/finally` 兜底 `EndVertical`）。
- 可空 `Directory.GetParent` 全部走 `ProjectRootPath()` 助手（null fallback `Application.dataPath`）。

## 家里机编译可能暴露的点（concerns）

1. **asmdef 引用**：`LoomGUI.Editor` references `LoomGUI.Runtime`——当前文件实际未用 Runtime 类型（LoomPackageSettings/PackageEntry/PkgManifestReader 都在 Editor 程序集）。引用留着是为将来面板扩展（如配 SpriteAtlas）。若编译报"未使用引用"警告（不会报错），可删 references。预期不报错。
2. **`GetWindow<T>(bool focus, string title, bool utility)`** 重载签名：Unity 各版本一致，但若家里机 Unity 版本极旧可能签名微调。低概率。
3. **`EditorStyles.wordWrappedMiniLabel`**：Unity 2019+ 有，家里机版本应有。若无该字段编译报错，换 `EditorStyles.wordWrappedLabel`。
4. **`ProcessStartInfo.StandardErrorEncoding`**：Unity 2019+（.NET 4.x）有。低风险。
5. **拖放区 `Event.current.Use()` + `GUIUtility.ExitGUI()`**：标准 IMGUI 模式，但 `ExitGUI` 抛 `ExitGUIException`——若家里机 Unity 版本对异常处理有差异可能行为不同。低风险（这是 Unity 官方推荐模式）。
6. **`Path.GetFullPath` 跨平台**：Windows 反斜杠已处理（`Replace('\\', '/')`），macOS/Linux 家里机（若有）正斜杠天然兼容。
7. **`PkgManifestReader` 解析正确性**：本机无法跑（无 Unity + 无 pkg.bin 样本）。逻辑对照 Rust `read_package` 逐段核对过（Header/StringTable/ComponentTable/NodeBlock skip/DynamicRules skip/AssetManifest），但家里机首次打包后校验才能验证。若 manifest 读偏（NodeBlock skip 长度算错），会抛 `PkgManifestException`（不崩，红字提示）——届时重核 skip 偏移即可。

## 未做 / 留 TODO

- **编辑器单测**：spec 说"编辑器面板无单测基建"，未建。家里机手验。
- **.meta 文件**：Unity 首次导入会自动生成 .meta（asmdef/cs）。本机不预生成（避免 GUID 冲突）。
