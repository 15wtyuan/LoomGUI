using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// v1.4-a T8：MirrorPool path→Sprite 查询冒烟测试（占位）。
    ///
    /// 旧 MirrorPoolTexIdTests（v4 blob + tex_id + texMap）随 T8 砍 tex_id/_texMap 一并退役。
    /// 新模型：blob v7 path_idx + path string table + SpriteResolver（path→Sprite）。
    /// 完整 round-trip 测试（path_idx→path→GetSprite→Sprite.texture+UV 重映射）见 T12：
    ///   - 手搓 v7 blob（path_idx + path table）+ mock SpriteResolver（注入 Sprite）
    ///   - 断言 Mr.sharedMaterial.mainTexture == Sprite.texture + Mesh.uv 重映射到 sprite.rect 子区
    /// 本 task 仅保证编译通过 + Sync 不崩（fallback 路径）。
    public class MirrorPoolPathTests
    {
        /// 空 pool.Sync 不崩（null SpriteResolver → fallback 路径）。占位冒烟。
        [Test]
        public void Sync_WithNullSpriteResolver_DoesNotCrash()
        {
            var root = new GameObject("root");
            root.transform.localScale = new Vector3(1f, -1f, 1f);
            var shader = Shader.Find("LoomGUI/Unlit");
            var mm = new MaterialManager(shader);
            var pool = new MirrorPool();
            try
            {
                // 8 字节全 0 buf：magic 不匹配 → IsValid=false → Sync 早退（不崩）。
                // 注：空 buf 会让 ReadU32(0) 越界抛异常，故用 8B（够读 magic+version 两 u32）。
                var blob = new FrameBlob(new byte[8]);
                Assert.IsFalse(blob.IsValid, "全 0 buf 不 IsValid（magic 不匹配）");
                pool.Sync(blob, root.transform, mm, null, Texture2D.whiteTexture, null);
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
