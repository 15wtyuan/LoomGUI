using System;
using System.Collections.Generic;
using System.Text;

namespace LoomGUI.Editor
{
    /// <summary>
    /// v1.4-a T9：从 pkg.bin 末尾的 AssetManifest 段读出 (path, w, h) 列表，供 Package Manager 面板
    /// 校验"manifest 里的 path 在 Unity res 目录下都有对应资源"。
    ///
    /// pkg.bin 布局（loomgui_core/src/asset/mod.rs，version=12）：
    ///   Header(20B): magic(u32 LE) + version(u32 LE) + flags(u32) + component_count(u32) + string_count(u32)
    ///   StringTable: string_count × {len(u16 LE) + utf8 bytes}
    ///   ComponentTable: component_count × {name_idx(u16) + root_node_idx(u32) + node_count(u32) + dynamic_len(u32)}
    ///   NodeBlock: total_nodes × 变长记录（含 style_len 前缀，可跳过）
    ///   PerComponentDynamicRules: component_count × dynamic_len 字节（按 ComponentTable 序）
    ///   AssetManifest: entry_count(u32) + entry_count × {path_idx(u16) + w(u32) + h(u32)}  ← 末尾
    ///
    /// 策略：读 Header + StringTable + ComponentTable → 跳 NodeBlock（逐节点按 style_len 跳）
    /// → 跳 DynamicRules（按 comp_table dynamic_len 之和跳）→ 读 AssetManifest，path 查 StringTable。
    /// 不引 bincode——只读 manifest，不解析 ResolvedStyle/DynamicRuleTable。
    ///
    /// 失败（magic/version 错、截断）抛 PkgManifestException，面板显示红字。
    /// </summary>
    public static class PkgManifestReader
    {
        public const uint PKG_MAGIC = 0x474B504C; // LE 字节 "LPKG"
        public const uint PKG_VERSION = 12;

        public sealed class AssetEntry
        {
            public string path;
            public uint w;
            public uint h;
        }

        public sealed class PkgManifestException : Exception
        {
            public PkgManifestException(string msg) : base(msg) { }
        }

        /// 读 pkg.bin 的 AssetManifest 段，返回 path 列表（含 w/h，校验用）。
        public static List<AssetEntry> ReadAssetManifest(byte[] bytes)
        {
            if (bytes == null || bytes.Length < 20)
            {
                throw new PkgManifestException("pkg.bin 过短（< 20B header）");
            }
            var r = new BinReader(bytes);
            // Header
            uint magic = r.U32();
            if (magic != PKG_MAGIC)
            {
                throw new PkgManifestException($"bad magic 0x{magic:X8}（期望 0x{PKG_MAGIC:X8}，不是 loom 包）");
            }
            uint version = r.U32();
            if (version != PKG_VERSION)
            {
                throw new PkgManifestException($"version 不匹配：{version}（期望 {PKG_VERSION}，旧包须重打）");
            }
            uint flags = r.U32(); // 未用
            uint componentCount = r.U32();
            uint stringCount = r.U32();

            // StringTable
            var strings = new string[stringCount];
            for (int i = 0; i < (int)stringCount; i++)
            {
                ushort len = r.U16();
                strings[i] = r.Utf8(len);
            }

            // ComponentTable: 每组件 {name_idx(u16) + root_node_idx(u32) + node_count(u32) + dynamic_len(u32)} = 18B
            var compTable = new CompRecord[componentCount];
            ulong totalNodes = 0;
            ulong totalDynamicLen = 0;
            for (int i = 0; i < (int)componentCount; i++)
            {
                ushort nameIdx = r.U16();
                uint rootNodeIdx = r.U32();
                uint nodeCount = r.U32();
                uint dynamicLen = r.U32();
                compTable[i] = new CompRecord { nameIdx = nameIdx, rootNodeIdx = rootNodeIdx, nodeCount = nodeCount, dynamicLen = dynamicLen };
                totalNodes += nodeCount;
                totalDynamicLen += dynamicLen;
            }

            // NodeBlock：逐节点跳过。每节点：
            //   parent_idx(i32) + kind_tag(u8) + style_len(u32) + style_blob(style_len) + text_idx(u16) +
            //   src_idx(u16) + class_count(u16) + class_idx[](class_count*2) + id_idx(u16) + flags(u8) + tabindex(i32)
            for (ulong i = 0; i < totalNodes; i++)
            {
                r.Skip(4 + 1);            // parent_idx + kind_tag
                uint styleLen = r.U32();
                r.Skip(styleLen);          // style_blob
                r.Skip((uint)(2 + 2));    // text_idx + src_idx
                ushort classCount = r.U16();
                r.Skip((uint)classCount * 2); // class_idx[]
                r.Skip((uint)(2 + 1 + 4)); // id_idx + flags + tabindex
            }

            // PerComponentDynamicRules：按 ComponentTable 序逐段 dynamicLen 字节
            r.Skip(totalDynamicLen);

            // AssetManifest：entry_count(u32) + count × {path_idx(u16) + w(u32) + h(u32)}
            uint entryCount = r.U32();
            var manifest = new List<AssetEntry>((int)entryCount);
            for (int i = 0; i < (int)entryCount; i++)
            {
                ushort pathIdx = r.U16();
                uint w = r.U32();
                uint h = r.U32();
                string path = StringAt(strings, pathIdx);
                manifest.Add(new AssetEntry { path = path, w = w, h = h });
            }
            return manifest;
        }

        static string StringAt(string[] strings, ushort idx)
        {
            const ushort NULL_IDX = 0xFFFF;
            if (idx == NULL_IDX)
            {
                return "";
            }
            if (idx >= strings.Length)
            {
                throw new PkgManifestException($"manifest path_idx {idx} 越界（string_count={strings.Length}）");
            }
            return strings[idx];
        }

        struct CompRecord
        {
            public ushort nameIdx;
            public uint rootNodeIdx;
            public uint nodeCount;
            public uint dynamicLen;
        }

        /// 简易 LE 二进制读游标，越界抛 PkgManifestException。
        struct BinReader
        {
            readonly byte[] b;
            int pos;

            public BinReader(byte[] bytes) { b = bytes; pos = 0; }

            public uint U32()
            {
                if (pos + 4 > b.Length) throw new PkgManifestException($"截断 @ {pos}（读 u32）");
                uint v = (uint)(b[pos] | (b[pos + 1] << 8) | (b[pos + 2] << 16) | (b[pos + 3] << 24));
                pos += 4;
                return v;
            }

            public ushort U16()
            {
                if (pos + 2 > b.Length) throw new PkgManifestException($"截断 @ {pos}（读 u16）");
                ushort v = (ushort)(b[pos] | (b[pos + 1] << 8));
                pos += 2;
                return v;
            }

            public string Utf8(ushort len)
            {
                if (pos + len > b.Length) throw new PkgManifestException($"截断 @ {pos}（读 utf8 len={len}）");
                string s = Encoding.UTF8.GetString(b, pos, len);
                pos += len;
                return s;
            }

            public void Skip(ulong n)
            {
                // 防 n 溢出 int：分块校验
                ulong end = (ulong)pos + n;
                if (end > (ulong)b.Length) throw new PkgManifestException($"截断 @ {pos}（跳 {n} 字节到 {end}，len={b.Length}）");
                pos = (int)end;
            }

            public void Skip(uint n) => Skip((ulong)n);
        }
    }
}
