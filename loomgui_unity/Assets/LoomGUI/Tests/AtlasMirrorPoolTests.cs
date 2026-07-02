using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    /// v1.4-a T8：Atlas MirrorPool 测试占位（旧 tex_id/atlas 测试随 T8 砍 _texMap/tex_id 退役）。
    ///
    /// 旧 AtlasMirrorPoolTests 验「同 atlas 多 sprite 共享 Texture2D + 子区 UV 烘焙」，
    /// 依赖 v4 blob tex_id 列 + texMap。新模型（T8）改 path_idx + path string table + SpriteResolver：
    ///   - SpriteAtlas 内 Sprite 自带 texture + rect（子区 UV 由 MirrorPool.RemapMeshUvToSprite 算）
    ///   - 同 atlas 多 sprite 共享 texture 由 SpriteAtlas 保证（MaterialManager key 命中同 tex → 同 Material → batch）
    /// 完整 round-trip 测试（手搓 v7 blob + mock SpriteAtlas/Sprite）见 T12 统一重写。
    /// 本 task 仅保证编译通过。
    public class AtlasMirrorPoolPathTests
    {
        /// SpriteResolver 注册 + 查询冒烟（不依赖 blob，纯 path→Sprite 逻辑）。
        [Test]
        public void SpriteResolver_Empty_Returns_MissingSprite()
        {
            var resolver = new SpriteResolver();
            // 无 atlas 注册 + 无 MissingSprite → GetSprite 返 null（fallback 路径，不崩）。
            var sp = resolver.GetSprite("icons/skin.png");
            Assert.IsNull(sp, "无 atlas 注册 → GetSprite 返 null（MissingSprite 默认 null）");
        }

        /// SpriteResolver 注册 null atlas 不崩（防御）。
        [Test]
        public void SpriteResolver_RegisterNullAtlas_DoesNotCrash()
        {
            var resolver = new SpriteResolver();
            resolver.RegisterAtlas(null);
            resolver.RegisterAtlases(null);
            Assert.AreEqual(0, resolver.AtlasCount, "null atlas 不入列表");
        }
    }
}
