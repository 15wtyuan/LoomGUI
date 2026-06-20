using System.Collections.Generic;
using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// MirrorPool flatten 坐标契约测试（Task 2，§4.2）。
    ///
    /// 验：blob local_x/local_y 是**绝对 design 坐标**（layout/mod.rs::write_back 递归累加父 origin）。
    /// 故所有渲染 GO 必须**挂根 GO**（flatten），localPosition = 绝对值；
    /// 否则巢状 + 绝对 localPosition 会把父坐标**双计**（Phase 1 单节点 pid=-1 未暴露此 bug）。
    ///
    /// 约定：root scale=(1,-1,1) pos=(0,0,0)（简化断言；design→world 即 (dx,-dy,0)）。
    /// parent @ design (100,200)，child @ design (50,50)，child.parent_id = parent.id。
    ///
    /// Unity EditMode 无法在无 Unity CLI 的环境跑（本仓 CI 不跑 C#）；执行延后到 review/PlayMode。
    /// 故本测试的**断言数值必须手算可证**——见类内注释的 hand-computation。
    public class MirrorPoolFlattenTests
    {
        /// 构造一个 2 节点 Mesh blob（v3）。
        /// node[0]=parent：visible=1, payload_kind=1, parent_id=-1, design=(100,200), mesh quad (0,0)(w,0)(w,h)(0,h)。
        /// node[1]=child：visible=1, payload_kind=1, parent_id=parent.id, design=(50,50), mesh quad 同尺寸。
        /// 同一 mesh arena，两个 mesh entry 紧挨；node 0 mesh_off=0，node 1 mesh_off=meshLen。
        static byte[] TwoMeshNodeBlob(
            uint parentId, float px, float py,
            uint childId, float cx, float cy,
            float w, float h, uint sortKey)
        {
            var b = new List<byte>();

            // header: magic, version=3, node_count=2
            b.AddRange(System.BitConverter.GetBytes(0x4D4F4F4Cu));
            b.AddRange(System.BitConverter.GetBytes(3u));
            b.AddRange(System.BitConverter.GetBytes(2u));

            const int HeaderLen = 12 + 14 * 4 + 2 * 4 + 2 * 4 + 2 * 4; // = 92
            const int NodeCount = 2;
            int colOff = HeaderLen;
            int[] offs = new int[14];
            int[] elemSize = { 4, 4, 1, 4, 4, 4, 4, 4, 1, 4, 4, 4, 4, 4 };
            // SOA（列优先，镜像 blob.rs/FrameBlob）：每列跨 NodeCount×elemSize 字节，列内 node0/node1 紧挨。
            // 旧版按 1 节点 elemSize 递进 + AoS 写数据 → 多节点读串列（SetTriangles idx 错），已修。
            for (int i = 0; i < 14; i++) { offs[i] = colOff; colOff += NodeCount * elemSize[i]; }
            int arenaOff = colOff;

            // mesh arena：2 个 mesh，每个 4 verts / 6 idx（顶点 re-base 到本地：(0,0)(w,0)(w,h)(0,h)）。
            var arena = new List<byte>();
            int arenaStart = arena.Count;

            void AppendMesh(float mw, float mh)
            {
                arena.AddRange(System.BitConverter.GetBytes(4)); // vert_count
                arena.AddRange(System.BitConverter.GetBytes(6)); // idx_count
                AppendVert(arena, 0f, 0f);
                AppendVert(arena, mw, 0f);
                AppendVert(arena, mw, mh);
                AppendVert(arena, 0f, mh);
                AppendVert(arena, 0f, 0f);
                AppendVert(arena, 1f, 0f);
                AppendVert(arena, 1f, 1f);
                AppendVert(arena, 0f, 1f);
                for (int v = 0; v < 4; v++)
                {
                    arena.AddRange(System.BitConverter.GetBytes(1f));
                    arena.AddRange(System.BitConverter.GetBytes(1f));
                    arena.AddRange(System.BitConverter.GetBytes(1f));
                    arena.AddRange(System.BitConverter.GetBytes(1f));
                }
                arena.AddRange(System.BitConverter.GetBytes(0u));
                arena.AddRange(System.BitConverter.GetBytes(1u));
                arena.AddRange(System.BitConverter.GetBytes(2u));
                arena.AddRange(System.BitConverter.GetBytes(0u));
                arena.AddRange(System.BitConverter.GetBytes(2u));
                arena.AddRange(System.BitConverter.GetBytes(3u));
            }

            AppendMesh(w, h);  // parent mesh @ offset 0
            int parentMeshLen = arena.Count - arenaStart;
            int childMeshOff = arena.Count - arenaStart;
            AppendMesh(w, h);  // child mesh @ childMeshOff
            int childMeshLen = (arena.Count - arenaStart) - childMeshOff;
            int arenaLen = arena.Count - arenaStart;

            // 14 列 offset + mesh/text/clip 三 arena off+len
            foreach (var o in offs) b.AddRange(System.BitConverter.GetBytes(o));
            b.AddRange(System.BitConverter.GetBytes(arenaOff));              // mesh_arena_off
            b.AddRange(System.BitConverter.GetBytes(arenaLen));              // mesh_arena_len
            b.AddRange(System.BitConverter.GetBytes(arenaOff + arenaLen));   // text_arena_off
            b.AddRange(System.BitConverter.GetBytes(0u));                    // text_arena_len
            int clipOff = arenaOff + arenaLen;
            b.AddRange(System.BitConverter.GetBytes(clipOff));               // clip_table_off
            b.AddRange(System.BitConverter.GetBytes(4u));                    // clip_table_len（仅 clip_count）

            // 列数据 SOA（列优先，镜像 blob.rs/FrameBlob）：每列先 node0 再 node1。列序同 elemSize。
            // col 0 node_id
            b.AddRange(System.BitConverter.GetBytes(parentId));              // node0
            b.AddRange(System.BitConverter.GetBytes(childId));               // node1
            // col 1 parent_id
            b.AddRange(System.BitConverter.GetBytes(-1));                    // node0 无父
            b.AddRange(System.BitConverter.GetBytes((int)parentId));         // node1 链 parent
            // col 2 visible
            b.Add(1); b.Add(1);
            // col 3 alpha
            b.AddRange(System.BitConverter.GetBytes(1f));
            b.AddRange(System.BitConverter.GetBytes(1f));
            // col 4 sort_key
            b.AddRange(System.BitConverter.GetBytes(sortKey));
            b.AddRange(System.BitConverter.GetBytes(sortKey));
            // col 5 local_x
            b.AddRange(System.BitConverter.GetBytes(px));                    // node0
            b.AddRange(System.BitConverter.GetBytes(cx));                    // node1
            // col 6 local_y
            b.AddRange(System.BitConverter.GetBytes(py));
            b.AddRange(System.BitConverter.GetBytes(cy));
            // col 7 mask_context
            b.AddRange(System.BitConverter.GetBytes(0u));
            b.AddRange(System.BitConverter.GetBytes(0u));
            // col 8 payload_kind
            b.Add(1); b.Add(1);                                              // Mesh
            // col 9 mesh_off
            b.AddRange(System.BitConverter.GetBytes(0u));                    // node0
            b.AddRange(System.BitConverter.GetBytes((uint)childMeshOff));    // node1
            // col 10 mesh_len
            b.AddRange(System.BitConverter.GetBytes((uint)parentMeshLen));
            b.AddRange(System.BitConverter.GetBytes((uint)childMeshLen));
            // col 11 text_off
            b.AddRange(System.BitConverter.GetBytes(0u));
            b.AddRange(System.BitConverter.GetBytes(0u));
            // col 12 text_len
            b.AddRange(System.BitConverter.GetBytes(0u));
            b.AddRange(System.BitConverter.GetBytes(0u));
            // col 13 tex_id（v3 新增，列优先：node0 再 node1；本测 0 占位）
            b.AddRange(System.BitConverter.GetBytes(0u));                    // node0
            b.AddRange(System.BitConverter.GetBytes(0u));                    // node1

            b.AddRange(arena);
            b.AddRange(System.BitConverter.GetBytes(0u));                    // clip_count = 0
            return b.ToArray();

            static void AppendVert(List<byte> a, float vx, float vy)
            {
                a.AddRange(System.BitConverter.GetBytes(vx));
                a.AddRange(System.BitConverter.GetBytes(vy));
            }
        }

        /// flatten：child 的 world position 不被父偏移。
        ///
        /// Hand-computation（root scale=(1,-1,1), pos=(0,0,0)）：
        ///   design (dx,dy) → world = root.TransformPoint(dx,dy,0) = (dx, -dy, 0)。
        ///   child design (50,50) → world (50,-50,0)。   ← flatten（正确）
        ///   旧巢状：child world = parent_go.TransformPoint(50,50,0)
        ///                       = parent.world(100,-200) + (50,-50) = (150,-250,0)。 ← 双计父（错）
        [Test]
        public void Flatten_ChildWorldPos_NotOffsetByParent()
        {
            var root = new GameObject("root");
            root.transform.localScale = new Vector3(1f, -1f, 1f);  // design→world: (dx,-dy,0)
            root.transform.position = Vector3.zero;
            var shader = Shader.Find("LoomGUI/Unlit");
            var mm = new MaterialManager(shader);
            var pool = new MirrorPool();
            var tex = Texture2D.whiteTexture;

            try
            {
                var blob = new FrameBlob(TwoMeshNodeBlob(
                    parentId: 7, px: 100f, py: 200f,
                    childId: 8, cx: 50f, cy: 50f,
                    w: 5f, h: 5f, sortKey: 1));
                Assert.AreEqual(2, blob.NodeCount, "blob 应解析出 2 节点");
                // T4：Sync 加 Font 参数（kind=2 用）；本测纯 Mesh，传 null。
                pool.Sync(blob, root.transform, mm, tex, null);

                // 找到 child GO（按 node_id 8 不能直接查 pool 内部 dict；遍历 root 直接子节点）。
                Assert.AreEqual(2, pool.Count, "flatten: 2 节点都应在 pool");
                Assert.AreEqual(2, root.transform.childCount,
                    "flatten: 2 节点都应是 root 直接子（不巢状）");

                Transform parentGo = null, childGo = null;
                for (int i = 0; i < root.transform.childCount; i++)
                {
                    var t = root.transform.GetChild(i);
                    // 无公开 API 直接查 node_id→GO；用 Sync 的两 GO 名都叫 loom_node，
                    // 故按 localPosition 反查（parent design (100,200)→local (100,200,0)；child (50,50,0)）。
                    var lp = t.localPosition;
                    if (Mathf.Approximately(lp.x, 100f) && Mathf.Approximately(lp.y, 200f)) parentGo = t;
                    else if (Mathf.Approximately(lp.x, 50f) && Mathf.Approximately(lp.y, 50f)) childGo = t;
                }
                Assert.IsNotNull(parentGo, "应找到 parent GO（localPosition (100,200,0)）");
                Assert.IsNotNull(childGo, "应找到 child GO（localPosition (50,50,0)）");

                // ① flatten：两 GO 都是 root 直接子（不巢状）
                Assert.AreSame(root.transform, parentGo.parent, "parent 应挂 root");
                Assert.AreSame(root.transform, childGo.parent, "flatten: child 也应挂 root（非 parent）");

                // ② flatten：child world = root.TransformPoint(50,50,0) = (50,-50,0)
                var expected = root.transform.TransformPoint(50f, 50f, 0f);
                Assert.AreEqual(expected, childGo.position,
                    "flatten: child world == root.TransformPoint(design(50,50))");

                // ③ 显式数值锁：root scale (1,-1,1) pos 0 → design (50,50) → world (50,-50,0)
                Assert.AreEqual(new Vector3(50f, -50f, 0f), childGo.position,
                    "flatten: child world 应精确为 (50,-50,0)");

                // ④ 反向锁：断言**不是**旧巢状的双计值 (150,-250,0)
                Assert.AreNotEqual(new Vector3(150f, -250f, 0f), childGo.position,
                    "flatten: child world 不能是巢状双计的 (150,-250,0)");

                // ⑤ parent 自己也应在绝对位（root.TransformPoint(100,200) = (100,-200,0)）
                Assert.AreEqual(new Vector3(100f, -200f, 0f), parentGo.position,
                    "parent world 应为 (100,-200,0)");
            }
            finally
            {
                pool.Clear();
                mm.Clear();
                Object.DestroyImmediate(root);
            }
        }
    }
}
