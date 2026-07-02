using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text;
using UnityEditor;
using UnityEngine;

namespace LoomGUI.Editor
{
    /// <summary>
    /// v1.4-a T9：Unity 包管理编辑器面板（spec §6.2）。
    /// 菜单 LoomGUI > Package Manager 打开。
    ///
    /// 功能：
    ///   1. 智能识别：拖目录到包列表 → 自动建包（pkgName=目录名 + 扫顶层 .html，不递归，排除 res）。
    ///   2. 包列表编辑：pkgName 可改、htmlFiles 增删、嵌套子目录 html 可单独拖入。
    ///   3. 每包刷新按钮：重扫 sourceDir 顶层 .html，增量更新（新增加入、删除移除），保留手动编辑。
    ///   4. 全局配置：resDirName + pkgOutputDir + loomPkgExePath（存 LoomPackageSettings.asset）。
    ///   5. 一键打包：对每包起 loomgui_pkg CLI 子进程，显示日志。
    ///   6. 资源校验：读 pkg.bin AssetManifest，校验 manifest 里的 path 在 res 下有对应文件，缺红字。
    /// </summary>
    public sealed class LoomPackageManagerWindow : EditorWindow
    {
        LoomPackageSettings _settings;
        Vector2 _pkgListScroll;
        Vector2 _logScroll;
        StringBuilder _log = new StringBuilder();
        GUIStyle _logStyle;

        [MenuItem("LoomGUI/Package Manager")]
        public static void Open()
        {
            var w = GetWindow<LoomPackageManagerWindow>(false, "Loom Package Manager", true);
            w.minSize = new Vector2(720, 480);
        }

        void OnEnable()
        {
            _settings = LoomPackageSettings.GetOrCreateDefault();
            _logStyle = null;
        }

        void InitStyles()
        {
            if (_logStyle == null)
            {
                _logStyle = new GUIStyle(EditorStyles.wordWrappedMiniLabel)
                {
                    richText = true,
                    fontSize = 11,
                };
            }
        }

        void OnGUI()
        {
            InitStyles();
            if (_settings == null)
            {
                _settings = LoomPackageSettings.GetOrCreateDefault();
            }

            // 拖放区（智能识别）：拖目录进来 → 扫顶层 .html 建包
            DrawDropZone();

            EditorGUILayout.Space(8);

            // 全局配置
            DrawGlobalConfig();

            EditorGUILayout.Space(8);

            // 包列表
            DrawPackageList();

            EditorGUILayout.Space(8);

            // 一键打包 / 资源校验按钮
            DrawActionButtons();

            EditorGUILayout.Space(8);

            // 日志
            DrawLog();
        }

        // ── 智能识别：拖目录 → 建包 ─────────────────────────────────────────

        void DrawDropZone()
        {
            Rect dropRect = GUILayoutUtility.GetRect(0, 56, GUILayout.ExpandWidth(true));
            GUI.Box(dropRect, "拖入系统目录（含 .html + res）以智能识别建包…\npkgName = 目录名，htmlFiles = 顶层 *.html（不递归，排除 res）", EditorStyles.helpBox);

            // 处理拖放
            if (dropRect.Contains(Event.current.mousePosition) && Event.current.type == EventType.DragPerform)
            {
                DragAndDrop.AcceptDrag();
                foreach (string path in DragAndDrop.paths)
                {
                    if (Directory.Exists(path))
                    {
                        SmartRecognizeDir(path);
                    }
                }
                Event.current.Use();
                GUIUtility.ExitGUI(); // 重绘
            }
            if (dropRect.Contains(Event.current.mousePosition) && Event.current.type == EventType.DragUpdated)
            {
                bool hasDir = false;
                foreach (string p in DragAndDrop.paths)
                {
                    if (Directory.Exists(p)) { hasDir = true; break; }
                }
                DragAndDrop.visualMode = hasDir ? DragAndDropVisualMode.Copy : DragAndDropVisualMode.Rejected;
            }
        }

        /// 智能识别：拖目录 → pkgName=目录名 + 扫顶层 .html（不递归，排除 res 目录）。
        void SmartRecognizeDir(string absDir)
        {
            string dirName = Path.GetFileName(absDir.TrimEnd('/', '\\'));
            if (string.IsNullOrEmpty(dirName))
            {
                AppendLog($"[skip] 目录名空: {absDir}");
                return;
            }
            // 转 Unity 工程相对路径（sourceDir 存相对，便于跨机器）
            string sourceDir = ToUnityRelativePath(absDir);
            if (sourceDir == null)
            {
                AppendLog($"[skip] 目录不在 Unity 工程内: {absDir}（需在 Assets/ 下或可相对工程根定位）");
                return;
            }
            var entry = new PackageEntry(dirName, sourceDir);
            // 扫顶层 .html（不递归，排除 res）
            entry.htmlFiles = ScanTopLevelHtml(absDir, _settings.resDirName);
            _settings.packages.Add(entry);
            MarkSettingsDirty();
            AppendLog($"[+] 智能识别建包: {entry.pkgName}（sourceDir={entry.sourceDir}，html ×{entry.htmlFiles.Count}）");
        }

        // ── 全局配置 ───────────────────────────────────────────────────────

        void DrawGlobalConfig()
        {
            EditorGUILayout.LabelField("全局配置", EditorStyles.boldLabel);
            EditorGUI.BeginChangeCheck();
            string resDir = EditorGUILayout.TextField("res 目录名", _settings.resDirName);
            string outDir = EditorGUILayout.TextField("pkg.bin 输出目录", _settings.pkgOutputDir);
            string exePath = EditorGUILayout.TextField("loomgui_pkg.exe 路径", _settings.loomPkgExePath);
            if (EditorGUI.EndChangeCheck())
            {
                Undo.RecordObject(_settings, "Edit LoomPackageSettings");
                _settings.resDirName = resDir;
                _settings.pkgOutputDir = outDir;
                _settings.loomPkgExePath = exePath;
                MarkSettingsDirty();
            }
            if (GUILayout.Button("打开 Settings 资产（Inspector 完整编辑）", GUILayout.Width(280)))
            {
                Selection.activeObject = _settings;
            }
        }

        // ── 包列表 ─────────────────────────────────────────────────────────

        void DrawPackageList()
        {
            EditorGUILayout.LabelField("包列表（" + _settings.packages.Count + "）", EditorStyles.boldLabel);

            _pkgListScroll = EditorGUILayout.BeginScrollView(_pkgListScroll, GUILayout.ExpandHeight(true));
            for (int i = 0; i < _settings.packages.Count; i++)
            {
                DrawPackageEntry(i);
            }
            EditorGUILayout.EndScrollView();

            if (GUILayout.Button("+ 手动添加空包", GUILayout.Width(160)))
            {
                Undo.RecordObject(_settings, "Add package");
                _settings.packages.Add(new PackageEntry("new_pkg", ""));
                MarkSettingsDirty();
            }
        }

        void DrawPackageEntry(int idx)
        {
            var pkg = _settings.packages[idx];
            EditorGUILayout.BeginVertical(EditorStyles.helpBox);
            try
            {
                EditorGUILayout.BeginHorizontal();
                EditorGUI.BeginChangeCheck();
                string name = EditorGUILayout.TextField("包名", pkg.pkgName, GUILayout.Width(220));
                string srcDir = EditorGUILayout.TextField("源目录", pkg.sourceDir);
                if (EditorGUI.EndChangeCheck())
                {
                    Undo.RecordObject(_settings, "Edit package");
                    pkg.pkgName = name;
                    pkg.sourceDir = srcDir;
                    MarkSettingsDirty();
                }
                if (GUILayout.Button("刷新", GUILayout.Width(60)))
                {
                    RefreshPackage(idx);
                }
                if (GUILayout.Button("校验", GUILayout.Width(60)))
                {
                    ValidatePackage(idx);
                }
                if (GUILayout.Button("打包", GUILayout.Width(60)))
                {
                    PackPackage(idx);
                }
                if (GUILayout.Button("删除", GUILayout.Width(60)))
                {
                    EditorGUILayout.EndHorizontal();
                    Undo.RecordObject(_settings, "Remove package");
                    _settings.packages.RemoveAt(idx);
                    MarkSettingsDirty();
                    GUIUtility.ExitGUI();
                    return;
                }
                EditorGUILayout.EndHorizontal();

                // htmlFiles 列表
                EditorGUILayout.LabelField("html 文件（" + pkg.htmlFiles.Count + "）:");
                for (int j = 0; j < pkg.htmlFiles.Count; j++)
                {
                    EditorGUILayout.BeginHorizontal();
                    EditorGUI.BeginChangeCheck();
                    string hf = EditorGUILayout.TextField(pkg.htmlFiles[j]);
                    if (EditorGUI.EndChangeCheck())
                    {
                        Undo.RecordObject(_settings, "Edit htmlFiles");
                        pkg.htmlFiles[j] = hf;
                        MarkSettingsDirty();
                    }
                    if (GUILayout.Button("×", GUILayout.Width(24)))
                    {
                        EditorGUILayout.EndHorizontal();
                        Undo.RecordObject(_settings, "Remove htmlFile");
                        pkg.htmlFiles.RemoveAt(j);
                        MarkSettingsDirty();
                        GUIUtility.ExitGUI();
                        return;
                    }
                    EditorGUILayout.EndHorizontal();
                }
                if (GUILayout.Button("+ 添加 html", GUILayout.Width(100)))
                {
                    Undo.RecordObject(_settings, "Add htmlFile");
                    pkg.htmlFiles.Add("");
                    MarkSettingsDirty();
                }

                // 嵌套子目录 html 手动补拖：单个 html 文件拖到本区域
                Rect htmlDrop = GUILayoutUtility.GetRect(0, 20, GUILayout.ExpandWidth(true));
                GUI.Box(htmlDrop, "  或拖单个 .html 文件到此补入（嵌套子目录的）", EditorStyles.miniLabel);
                if (htmlDrop.Contains(Event.current.mousePosition) && Event.current.type == EventType.DragPerform)
                {
                    DragAndDrop.AcceptDrag();
                    foreach (string p in DragAndDrop.paths)
                    {
                        if (!string.IsNullOrEmpty(p) && p.EndsWith(".html", StringComparison.OrdinalIgnoreCase))
                        {
                            string rel = HtmlRelativeToSource(p, pkg.sourceDir);
                            if (rel != null && !pkg.htmlFiles.Contains(rel))
                            {
                                Undo.RecordObject(_settings, "Drop html");
                                pkg.htmlFiles.Add(rel);
                                MarkSettingsDirty();
                            }
                        }
                    }
                    Event.current.Use();
                }
                if (htmlDrop.Contains(Event.current.mousePosition) && Event.current.type == EventType.DragUpdated)
                {
                    bool ok = false;
                    foreach (string p in DragAndDrop.paths)
                    {
                        if (p.EndsWith(".html", StringComparison.OrdinalIgnoreCase)) { ok = true; break; }
                    }
                    DragAndDrop.visualMode = ok ? DragAndDropVisualMode.Copy : DragAndDropVisualMode.Rejected;
                }
            }
            finally
            {
                EditorGUILayout.EndVertical();
            }
        }

        // ── 刷新（增量同步）────────────────────────────────────────────────

        /// 刷新：重扫 sourceDir 顶层 .html，diff htmlFiles：
        ///   - 新增（目录里有、列表里没有）→ 加入
        ///   - 删除（列表里有、目录里没有）→ 移除
        ///   - 保留：手动加的非目录内 html（不在扫描结果里但用户显式加的，保留）
        ///   - 保留：改过的 pkgName（刷新只动 htmlFiles，不动 pkgName）
        void RefreshPackage(int idx)
        {
            var pkg = _settings.packages[idx];
            string abs = ToProjectAbsolutePath(pkg.sourceDir);
            if (abs == null || !Directory.Exists(abs))
            {
                AppendLog($"[refresh] {pkg.pkgName}: sourceDir 不存在 ({pkg.sourceDir})");
                return;
            }
            var scanned = ScanTopLevelHtml(abs, _settings.resDirName);
            var scannedSet = new HashSet<string>(scanned);
            var existing = new HashSet<string>(pkg.htmlFiles);

            Undo.RecordObject(_settings, "Refresh package");
            // 新增加入
            int added = 0;
            foreach (var s in scanned)
            {
                if (!existing.Contains(s))
                {
                    pkg.htmlFiles.Add(s);
                    added++;
                }
            }
            // 删除移除（只移扫描集里曾存在但现在不在的——即原本是扫描来的、现在目录里没了的）
            // 启发：若一个 html 在扫描集里（文件名像顶层 .html）且目录现在扫不到 → 移除。
            // 但无法区分"手动加的顶层 html 名"vs"扫描来的"。约定：手动补拖的通常是子目录 html
            //（带路径分隔符），顶层 html 名无分隔符 → 用"无分隔符且不在新扫描集"判删除。
            int removed = 0;
            for (int j = pkg.htmlFiles.Count - 1; j >= 0; j--)
            {
                string hf = pkg.htmlFiles[j];
                bool isTopLevel = hf.IndexOfAny(new[] { '/', '\\' }) < 0;
                if (isTopLevel && !scannedSet.Contains(hf))
                {
                    pkg.htmlFiles.RemoveAt(j);
                    removed++;
                }
            }
            MarkSettingsDirty();
            AppendLog($"[refresh] {pkg.pkgName}: +{added} 新增 / -{removed} 移除 / 保留 {pkg.htmlFiles.Count} 个");
        }

        // ── 资源校验 ───────────────────────────────────────────────────────

        /// 资源校验：读 pkg.bin AssetManifest，校验 manifest 里每个 path 在 sourceDir/res 下有对应文件。
        /// 缺失 → 红字列出。
        void ValidatePackage(int idx)
        {
            var pkg = _settings.packages[idx];
            string pkgBinPath = PkgBinOutputPath(pkg);
            if (!File.Exists(pkgBinPath))
            {
                AppendLog($"[validate] {pkg.pkgName}: pkg.bin 不存在 ({pkgBinPath})，请先打包");
                return;
            }
            try
            {
                byte[] bytes = File.ReadAllBytes(pkgBinPath);
                var manifest = PkgManifestReader.ReadAssetManifest(bytes);
                string absSrc = ToProjectAbsolutePath(pkg.sourceDir);
                string resRoot = absSrc != null ? Path.Combine(absSrc, _settings.resDirName) : null;
                int missing = 0;
                foreach (var e in manifest)
                {
                    if (string.IsNullOrEmpty(e.path)) continue;
                    if (resRoot == null) continue;
                    // path 是相对 res 的归一化路径（正斜杠），拼绝对路径校验文件存在
                    string abs = Path.Combine(resRoot, e.path.Replace('/', Path.DirectorySeparatorChar));
                    if (!File.Exists(abs))
                    {
                        AppendLog($"[validate] {pkg.pkgName}: <color=red>缺资源 {e.path}</color>");
                        missing++;
                    }
                }
                if (missing == 0)
                {
                    AppendLog($"[validate] {pkg.pkgName}: <color=#2a8a2a>OK，{manifest.Count} 条 manifest path 全齐</color>");
                }
                else
                {
                    AppendLog($"[validate] {pkg.pkgName}: <color=red>缺 {missing}/{manifest.Count} 条资源</color>");
                }
            }
            catch (Exception e)
            {
                AppendLog($"[validate] {pkg.pkgName}: <color=red>读 pkg.bin 失败 — {e.Message}</color>");
            }
        }

        // ── 一键打包 ───────────────────────────────────────────────────────

        void DrawActionButtons()
        {
            EditorGUILayout.BeginHorizontal();
            if (GUILayout.Button("一键打包全部", GUILayout.Height(28), GUILayout.Width(160)))
            {
                PackAll();
            }
            if (GUILayout.Button("校验全部", GUILayout.Height(28), GUILayout.Width(120)))
            {
                for (int i = 0; i < _settings.packages.Count; i++) ValidatePackage(i);
            }
            if (GUILayout.Button("清空日志", GUILayout.Height(28), GUILayout.Width(100)))
            {
                _log.Clear();
            }
            EditorGUILayout.EndHorizontal();
        }

        void PackAll()
        {
            if (_settings.packages.Count == 0)
            {
                AppendLog("[pack] 包列表为空");
                return;
            }
            for (int i = 0; i < _settings.packages.Count; i++)
            {
                PackPackage(i);
            }
        }

        /// 打包单包：起 loomgui_pkg CLI 子进程。
        /// args = "<sourceDir> <pkgName> --html <h1,h2,...> --res <name> -o <out>"
        void PackPackage(int idx)
        {
            var pkg = _settings.packages[idx];
            if (string.IsNullOrEmpty(pkg.pkgName) || string.IsNullOrEmpty(pkg.sourceDir))
            {
                AppendLog($"[pack] 包 #{idx}: pkgName/sourceDir 空，跳过");
                return;
            }
            string exe = ResolveExePath(_settings.loomPkgExePath);
            if (!File.Exists(exe))
            {
                AppendLog($"[pack] {pkg.pkgName}: <color=red>找不到 loomgui_pkg.exe ({exe})。请先 cargo build --release -p loomgui_pkg，或在配置里改 loomPkgExePath</color>");
                return;
            }
            string absSrc = ToProjectAbsolutePath(pkg.sourceDir);
            if (absSrc == null || !Directory.Exists(absSrc))
            {
                AppendLog($"[pack] {pkg.pkgName}: sourceDir 不存在 ({pkg.sourceDir})");
                return;
            }
            string outPath = PkgBinOutputPath(pkg);
            EnsureDirExists(outPath);

            // 拼 args
            string htmlArg = pkg.htmlFiles.Count > 0 ? string.Join(",", pkg.htmlFiles) : "";
            var sb = new StringBuilder();
            sb.Append('"').Append(absSrc).Append("\" ");
            sb.Append(pkg.pkgName);
            if (pkg.htmlFiles.Count > 0)
            {
                sb.Append(" --html ").Append(htmlArg);
            }
            sb.Append(" --res ").Append(_settings.resDirName);
            sb.Append(" -o \"").Append(outPath).Append('"');
            string args = sb.ToString();

            AppendLog($"[pack] {pkg.pkgName}: loomgui_pkg {args}");
            try
            {
                var psi = new ProcessStartInfo(exe, args)
                {
                    RedirectStandardOutput = true,
                    RedirectStandardError = true,
                    UseShellExecute = false,
                    CreateNoWindow = true,
                    StandardOutputEncoding = Encoding.UTF8,
                    StandardErrorEncoding = Encoding.UTF8,
                };
                using (var p = Process.Start(psi))
                {
                    string stdout = p.StandardOutput.ReadToEnd();
                    string stderr = p.StandardError.ReadToEnd();
                    p.WaitForExit();
                    if (!string.IsNullOrEmpty(stdout)) AppendLog($"  stdout: {stdout.Trim()}");
                    if (!string.IsNullOrEmpty(stderr)) AppendLog($"  <color=yellow>stderr: {stderr.Trim()}</color>");
                    AppendLog($"  exit={p.ExitCode}");
                    if (p.ExitCode == 0)
                    {
                        AppendLog($"  <color=#2a8a2a>OK → {outPath}</color>");
                    }
                    else
                    {
                        AppendLog($"  <color=red>失败 exit={p.ExitCode}</color>");
                    }
                }
                AssetDatabase.Refresh();
            }
            catch (Exception e)
            {
                AppendLog($"[pack] {pkg.pkgName}: <color=red>启动子进程失败 — {e.Message}</color>");
            }
        }

        // ── 日志 ───────────────────────────────────────────────────────────

        void DrawLog()
        {
            EditorGUILayout.LabelField("日志", EditorStyles.boldLabel);
            _logScroll = EditorGUILayout.BeginScrollView(_logScroll, GUILayout.Height(160));
            EditorGUILayout.LabelField(_log.ToString(), _logStyle);
            EditorGUILayout.EndScrollView();
        }

        void AppendLog(string line)
        {
            _log.AppendLine(line);
            _logScroll.y = float.MaxValue; // 自动滚到底
        }

        // ── 工具方法 ───────────────────────────────────────────────────────

        /// 扫 sourceDir 顶层 .html（不递归，排除 res 目录）。
        static List<string> ScanTopLevelHtml(string absDir, string resDirName)
        {
            var list = new List<string>();
            if (string.IsNullOrEmpty(absDir) || !Directory.Exists(absDir)) return list;
            foreach (var f in Directory.GetFiles(absDir, "*.html", SearchOption.TopDirectoryOnly))
            {
                string name = Path.GetFileName(f);
                if (string.Equals(name, resDirName, StringComparison.OrdinalIgnoreCase)) continue;
                list.Add(name);
            }
            list.Sort(StringComparer.OrdinalIgnoreCase);
            return list;
        }

        /// sourceDir（Unity 相对，如 "Assets/LoomUI/Bag"）→ 绝对路径。
        static string ToProjectAbsolutePath(string unityRelative)
        {
            if (string.IsNullOrEmpty(unityRelative)) return null;
            string assetsRoot = Application.dataPath; // .../Assets
            // sourceDir 可能以 Assets/ 开头或工程根相对。去 Assets/ 前缀后拼到 assetsRoot，
            // 否则拼到工程根（Assets/..）。都走 Path.GetFullPath 规范化。
            string norm = unityRelative.Replace('\\', '/').TrimStart('/');
            const string assetsPrefix = "Assets/";
            bool underAssets = norm.StartsWith(assetsPrefix, StringComparison.OrdinalIgnoreCase);
            string baseDir = underAssets ? assetsRoot : ProjectRootPath();
            if (underAssets) norm = norm.Substring(assetsPrefix.Length);
            return Path.GetFullPath(Path.Combine(baseDir, norm));
        }

        /// 绝对路径 → Unity 工程相对路径（工程根相对）。不在工程内返回 null。
        static string ToUnityRelativePath(string abs)
        {
            string full = Path.GetFullPath(abs).Replace('\\', '/');
            string projRoot = ProjectRootPath().Replace('\\', '/').TrimEnd('/') + "/";
            if (!full.StartsWith(projRoot, StringComparison.OrdinalIgnoreCase)) return null;
            return full.Substring(projRoot.Length);
        }

        /// 拖入的 html 绝对路径 → 相对 sourceDir 的路径（用于 htmlFiles，支持嵌套子目录）。
        /// 不在 sourceDir 下 → 返回文件名（顶层场景）或 null。
        static string HtmlRelativeToSource(string htmlPath, string sourceDir)
        {
            string absHtml = Path.GetFullPath(htmlPath).Replace('\\', '/');
            string absSrc = ToProjectAbsolutePath(sourceDir);
            if (absSrc == null) return Path.GetFileName(htmlPath);
            absSrc = absSrc.Replace('\\', '/').TrimEnd('/') + "/";
            if (absHtml.StartsWith(absSrc, StringComparison.OrdinalIgnoreCase))
            {
                return absHtml.Substring(absSrc.Length);
            }
            return Path.GetFileName(htmlPath);
        }

        /// pkg.bin 输出路径 = pkgOutputDir/<pkgName>.pkg.bin（绝对，工程根相对）。
        string PkgBinOutputPath(PackageEntry pkg)
        {
            string outDir = (_settings.pkgOutputDir ?? "").Replace('\\', '/').TrimStart('/');
            string projRoot = ProjectRootPath();
            string abs = Path.GetFullPath(Path.Combine(projRoot, outDir));
            return Path.Combine(abs, pkg.pkgName + ".pkg.bin");
        }

        /// 工程根绝对路径（= Application.dataPath 的父，Assets/..）。
        static string ProjectRootPath()
        {
            var parent = Directory.GetParent(Application.dataPath);
            if (parent == null) return Application.dataPath;
            return parent.FullName;
        }

        /// 解析 exe 路径：工程相对或绝对。
        static string ResolveExePath(string configured)
        {
            if (string.IsNullOrEmpty(configured)) configured = "../target/release/loomgui_pkg.exe";
            string projRoot = ProjectRootPath();
            string candidate = Path.IsPathRooted(configured)
                ? configured
                : Path.GetFullPath(Path.Combine(projRoot, configured));
            // Windows 自动补 .exe（若未带）
            if (Application.platform == RuntimePlatform.WindowsEditor &&
                !candidate.EndsWith(".exe", StringComparison.OrdinalIgnoreCase))
            {
                candidate += ".exe";
            }
            return candidate;
        }

        static void EnsureDirExists(string filePath)
        {
            string dir = Path.GetDirectoryName(filePath);
            if (!string.IsNullOrEmpty(dir) && !Directory.Exists(dir))
            {
                Directory.CreateDirectory(dir);
            }
        }

        void MarkSettingsDirty()
        {
            EditorUtility.SetDirty(_settings);
        }
    }
}
