using System;
using UnityEngine;

namespace LoomGUI
{
    /// 帧 blob 托管解析视图（v2）。解析 Rust build_blob 产出的 little-endian blob。
    ///
    /// 布局（镜像 loomgui_ffi_c/src/blob.rs）：
    ///   header (88B): magic(u32 LE), version(u32)=2, node_count(u32),
    ///                 13× col_offset(u32, byte offset from blob start),
    ///                 mesh_arena_off(u32), mesh_arena_len(u32),
    ///                 text_arena_off(u32), text_arena_len(u32),   // v2 新增
    ///                 clip_table_off(u32), clip_table_len(u32)    // v2 新增
    ///   13 列 SOA（顺序见 ColOff 注释），随后 mesh_arena / text_arena / clip_table 段。
    /// C# on Windows 是 little-endian，BitConverter 直读无需 byte swap。
    public readonly struct FrameBlob
    {
        public const uint Magic = 0x4D4F4F4C;
        /// blob 版本（v2）。magic+version 校验在 IsValid。
        public const uint ExpectedVersion = 2;

        readonly byte[] _buf;

        public FrameBlob(byte[] buf) { _buf = buf; }

        /// magic==Magic && version==2。MirrorPool.Sync 顶据此拒绝陈旧/非 v2 blob。
        public bool IsValid => ReadU32(0) == Magic && ReadU32(4) == ExpectedVersion;
        public uint Version => ReadU32(4);
        public int NodeCount => (int)ReadU32(8);

        // 列 offset 在 header[12 .. 12+13*4)。顺序同 Rust columns：
        //   0=node_id(u32) 1=parent_id(i32,-1=none) 2=visible(u8) 3=alpha(f32)
        //   4=sort_key(u32) 5=local_x(f32) 6=local_y(f32) 7=mask_context(u32)
        //   8=payload_kind(u8, 0=Unchanged 1=Mesh 2=Text) 9=mesh_off(u32) 10=mesh_len(u32)
        //   11=text_off(u32) 12=text_len(u32)   （v2 新增）
        int ColOff(int idx) => (int)ReadU32(12 + idx * 4);
        // 三 arena header offset。13 列 col_offset 之后：mesh(2), text(2), clip(2) 各 off+len。
        // mesh_arena_off @ 12+13*4 = 64；mesh_arena_len @ 68。
        int MeshArenaOff => (int)ReadU32(12 + 13 * 4);
        // text_arena_off @ 12+13*4+2*4 = 72；text_arena_len @ 76。
        int TextArenaOff => (int)ReadU32(12 + 13 * 4 + 2 * 4);
        int TextArenaLen => (int)ReadU32(12 + 13 * 4 + 2 * 4 + 4);
        // clip_table_off @ 12+13*4+4*4 = 80；clip_table_len @ 84。
        int ClipTableOff => (int)ReadU32(12 + 13 * 4 + 4 * 4);
        int ClipTableLen => (int)ReadU32(12 + 13 * 4 + 4 * 4 + 4);

        public uint NodeId(int i) => ReadU32(ColOff(0) + i * 4);
        public int ParentId(int i) => (int)ReadU32(ColOff(1) + i * 4);
        public bool Visible(int i) => _buf[ColOff(2) + i] != 0;
        public float Alpha(int i) => ReadF32(ColOff(3) + i * 4);
        public uint SortKey(int i) => ReadU32(ColOff(4) + i * 4);
        public float LocalX(int i) => ReadF32(ColOff(5) + i * 4);
        public float LocalY(int i) => ReadF32(ColOff(6) + i * 4);
        public uint MaskContext(int i) => ReadU32(ColOff(7) + i * 4);
        public byte PayloadKind(int i) => _buf[ColOff(8) + i];
        uint MeshOff(int i) => ReadU32(ColOff(9) + i * 4);
        uint MeshLen(int i) => ReadU32(ColOff(10) + i * 4);
        // v2 新增（T3/T4 消费 text_arena）。
        public uint TextOff(int i) => ReadU32(ColOff(11) + i * 4);
        public uint TextLen(int i) => ReadU32(ColOff(12) + i * 4);

        /// clip 表 entry 数（context>0 入表；T5 填）。T1 恒为 0。
        /// clip 表段布局：clip_count(u32) + entries[count × {ctx,x,y,w,h}]。
        /// clip_count(u32) 在 ClipTableOff 处；clip_table_len(header @84) 含 clip_count 本身。
        public int ClipCount => ClipTableLen >= 4 ? (int)ReadU32(ClipTableOff) : 0;

        /// 读某 clip context 的 design rect（绝对，y-down）。entry 布局：ctx,x,y,w,h 各 4B（20B/entry）。
        /// mask_context==0 永不入表（无裁剪）；未找到 ctx → found=false（调用方跳过 SetClipBox）。
        /// 镜像 Rust blob.rs::read_clips。线性扫描（few entries，O(n) 足够）。
        public bool ClipRect(uint ctx, out float x, out float y, out float w, out float h)
        {
            int count = ClipCount;
            int p = ClipTableOff + 4;   // 跳过 clip_count
            for (int i = 0; i < count; i++)
            {
                if (ReadU32(p) == ctx)
                {
                    x = ReadF32(p + 4);
                    y = ReadF32(p + 8);
                    w = ReadF32(p + 12);
                    h = ReadF32(p + 16);
                    return true;
                }
                p += 20;
            }
            x = y = w = h = 0f;
            return false;
        }

        /// 读节点 i 的 mesh（仅 payload_kind==1 时调用）。
        /// mesh arena 段布局：vert_count(u32) idx_count(u32) verts[vc×2 f32] uvs[vc×2 f32]
        ///               colors[vc×4 f32] indices[idx_count u32]。
        /// 返回的 MeshSegment 持有 verts/uvs/colors/indices 数组的拷贝。
        public MeshSegment ReadMesh(int i)
        {
            int p = MeshArenaOff + (int)MeshOff(i);
            int vertCount = (int)ReadU32(p); p += 4;
            int idxCount = (int)ReadU32(p); p += 4;
            var seg = new MeshSegment(vertCount, idxCount);
            for (int v = 0; v < vertCount; v++)
            {
                seg.Verts[v] = new UnityEngine.Vector2(ReadF32(p), ReadF32(p + 4)); p += 8;
            }
            for (int v = 0; v < vertCount; v++)
            {
                seg.Uvs[v] = new UnityEngine.Vector2(ReadF32(p), ReadF32(p + 4)); p += 8;
            }
            for (int v = 0; v < vertCount; v++)
            {
                seg.Colors[v] = new UnityEngine.Color(ReadF32(p), ReadF32(p + 4), ReadF32(p + 8), ReadF32(p + 12)); p += 16;
            }
            for (int k = 0; k < idxCount; k++) { seg.Idx[k] = ReadU32(p); p += 4; }
            return seg;
        }

        /// 读节点 i 的 text 段（仅 payload_kind==2 时调用）。镜像 Rust blob.rs::read_text。
        /// per-node 段布局（little-endian，§4.1/§4.3）：
        ///   font_size:u32 | color:f32×4 | glyph_count:u32
        ///   | glyphs[count × { codepoint:u32, pen_x:f32, pen_y:f32 }]  (12B/glyph)
        /// pen_x/pen_y 已 GO-local 绝对 design（content 偏移在 render/mod.rs 烤进 glyph.x）；
        /// pen_y = line.y + line.baseline（绝对，同行同值）。Unity 不 re-base、不用 advance。
        public void ReadText(int i, out int fontSize, out Color color, out GlyphData[] glyphs)
        {
            int p = TextArenaOff + (int)TextOff(i);
            fontSize = (int)ReadU32(p); p += 4;
            float r = ReadF32(p); p += 4;
            float g = ReadF32(p); p += 4;
            float b = ReadF32(p); p += 4;
            float a = ReadF32(p); p += 4;
            color = new Color(r, g, b, a);
            int count = (int)ReadU32(p); p += 4;
            glyphs = new GlyphData[count];
            for (int k = 0; k < count; k++)
            {
                uint cp = ReadU32(p); p += 4;
                float px = ReadF32(p); p += 4;
                float py = ReadF32(p); p += 4;
                glyphs[k] = new GlyphData(cp, px, py);
            }
        }

        uint ReadU32(int o) => BitConverter.ToUInt32(_buf, o);
        float ReadF32(int o) => BitConverter.ToSingle(_buf, o);
    }

    /// 单 glyph 笔位（GO-local 绝对 design，y-down）。codepoint 为 Unicode 标量值（传 GetCharacterInfo
    /// 前 cast char；BMP 外暂不支持——T3 仅 ASCII/BMP 测，v1c emoji 再议）。
    public readonly struct GlyphData
    {
        public readonly uint Codepoint;
        public readonly float PenX;
        public readonly float PenY;
        public GlyphData(uint codepoint, float penX, float penY)
        {
            Codepoint = codepoint; PenX = penX; PenY = penY;
        }
    }

    /// ReadMesh 返回的 mesh 数据拷贝。verts/uvs/colors 长度 == vertCount，Idx 长度 == idxCount。
    public sealed class MeshSegment
    {
        public readonly UnityEngine.Vector2[] Verts;
        public readonly UnityEngine.Vector2[] Uvs;
        public readonly UnityEngine.Color[] Colors;
        public readonly uint[] Idx;

        public MeshSegment(int vertCount, int idxCount)
        {
            Verts = new UnityEngine.Vector2[vertCount];
            Uvs = new UnityEngine.Vector2[vertCount];
            Colors = new UnityEngine.Color[vertCount];
            Idx = new uint[idxCount];
        }
    }
}
