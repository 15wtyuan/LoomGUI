using System;

namespace LoomGUI
{
    /// 帧 blob 托管解析视图（Task 4）。解析 Rust build_blob 产出的 little-endian blob。
    ///
    /// 布局（镜像 loomgui_ffi_c/src/blob.rs）：
    ///   header (64B): magic(u32 LE), version(u32), node_count(u32),
    ///                 11× col_offset(u32, byte offset from blob start),
    ///                 arena_off(u32), arena_len(u32)
    ///   11 列 SOA（顺序见 ColOff 注释），随后 mesh arena 段。
    /// C# on Windows 是 little-endian，BitConverter 直读无需 byte swap。
    public readonly struct FrameBlob
    {
        public const uint Magic = 0x4D4F4F4C;

        readonly byte[] _buf;

        public FrameBlob(byte[] buf) { _buf = buf; }

        public int NodeCount => (int)ReadU32(8);

        // 列 offset 在 header[12 .. 12+11*4)。顺序同 Rust columns：
        //   0=node_id(u32) 1=parent_id(i32,-1=none) 2=visible(u8) 3=alpha(f32)
        //   4=sort_key(u32) 5=local_x(f32) 6=local_y(f32) 7=mask_context(u32)
        //   8=payload_kind(u8, 0=Unchanged 1=Mesh 2=Text) 9=mesh_off(u32) 10=mesh_len(u32)
        int ColOff(int idx) => (int)ReadU32(12 + idx * 4);
        // arena_off @ offset 12+11*4=56；arena_len @ offset 12+12*4=60。
        int ArenaOff => (int)ReadU32(12 + 11 * 4);
        int ArenaLen => (int)ReadU32(12 + 12 * 4);

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

        /// 读节点 i 的 mesh（仅 payload_kind==1 时调用）。
        /// arena 段布局：vert_count(u32) idx_count(u32) verts[vc×2 f32] uvs[vc×2 f32]
        ///               colors[vc×4 f32] indices[idx_count u32]。
        /// 返回的 MeshSegment 持有 verts/uvs/colors/indices 数组的拷贝。
        public MeshSegment ReadMesh(int i)
        {
            int p = ArenaOff + (int)MeshOff(i);
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

        uint ReadU32(int o) => BitConverter.ToUInt32(_buf, o);
        float ReadF32(int o) => BitConverter.ToSingle(_buf, o);
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
