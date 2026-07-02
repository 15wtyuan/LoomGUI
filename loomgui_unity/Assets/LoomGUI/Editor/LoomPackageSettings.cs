using System;
using System.Collections.Generic;
using UnityEditor;
using UnityEngine;

namespace LoomGUI.Editor
{
    /// <summary>
    /// v1.4-a T9：包管理配置（工程内一份 ScriptableObject）。
    /// 菜单 LoomGUI > Package Settings 创建，或 Package Manager 面板自动建。
    ///
    /// 字段（spec §6.1）：
    ///   - resDirName：资源目录名（可配置，默认 res）。打包器按此前缀归一化 img path。
    ///   - pkgOutputDir：pkg.bin 输出目录（Unity 工程内相对路径，默认 Assets/StreamingAssets/）。
    ///   - packages：包列表（拖目录智能识别或手动增）。
    ///   - loomPkgExePath：loomgui_pkg.exe 绝对/工程相对路径（默认 ../target/release/loomgui_pkg.exe）。
    /// </summary>
    [CreateAssetMenu(menuName = "LoomGUI/Package Settings", fileName = "LoomPackageSettings")]
    public sealed class LoomPackageSettings : ScriptableObject
    {
        [Tooltip("资源目录名（打包器按此前缀归一化 img path，默认 res）")]
        public string resDirName = "res";

        [Tooltip("pkg.bin 输出目录（Unity 工程内相对路径）")]
        public string pkgOutputDir = "Assets/StreamingAssets/";

        [Tooltip("loomgui_pkg.exe 路径。可填工程相对路径（如 ../target/release/loomgui_pkg.exe）或绝对路径")]
        public string loomPkgExePath = "../target/release/loomgui_pkg.exe";

        [Tooltip("包列表（拖目录到 Package Manager 面板自动建，或手动加）")]
        public List<PackageEntry> packages = new List<PackageEntry>();

        /// 缺省配置资产路径（Package Manager 面板首次打开时若无配置自动建到这里）。
        public const string DefaultAssetPath = "Assets/LoomGUI/Editor/LoomPackageSettings.asset";

        /// 在工程内查找或创建默认配置资产。
        public static LoomPackageSettings GetOrCreateDefault()
        {
            var existing = AssetDatabase.LoadAssetAtPath<LoomPackageSettings>(DefaultAssetPath);
            if (existing != null)
            {
                return existing;
            }
            var asset = CreateInstance<LoomPackageSettings>();
            AssetDatabase.CreateAsset(asset, DefaultAssetPath);
            AssetDatabase.SaveAssets();
            return asset;
        }
    }

    /// <summary>
    /// 单个包配置（spec §6.1）。
    ///   - pkgName：包名（拖目录时默认=目录名，可改）。
    ///   - sourceDir：该包源目录（Unity 工程内相对路径，含 html + res）。
    ///   - htmlFiles：该包含的 html 文件名列表（顶层扫描 + 手动增删，相对 sourceDir）。
    /// </summary>
    [Serializable]
    public sealed class PackageEntry
    {
        [Tooltip("包名（拖目录时默认=目录名，可改）")]
        public string pkgName = "";

        [Tooltip("该包源目录（Unity 工程内相对路径，含 html + res）")]
        public string sourceDir = "";

        [Tooltip("该包含的 html 文件名列表（相对 sourceDir）")]
        public List<string> htmlFiles = new List<string>();

        public PackageEntry()
        {
        }

        public PackageEntry(string pkgName, string sourceDir)
        {
            this.pkgName = pkgName;
            this.sourceDir = sourceDir;
        }
    }
}
