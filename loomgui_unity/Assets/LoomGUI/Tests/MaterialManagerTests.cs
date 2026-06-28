using NUnit.Framework;
using UnityEngine;

namespace LoomGUI.Tests
{
    public class MaterialManagerTests
    {
        [Test]
        public void SameKeyReturnsSameMaterial()
        {
            var mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            var white = Texture2D.whiteTexture;
            var a = mm.Get(program: 0, white, maskContext: 0, false);
            var b = mm.Get(program: 0, white, maskContext: 0, false);
            Assert.AreSame(a, b);
        }

        [Test]
        public void DifferentMaskContextReturnsDifferentMaterial()
        {
            var mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            var white = Texture2D.whiteTexture;
            var a = mm.Get(0, white, 0, false);
            var b = mm.Get(0, white, 1, false);
            Assert.AreNotSame(a, b);
        }

        /// ctx>0 material 创建即 EnableKeyword("CLIPPED")（激活 shader multi_compile _ CLIPPED variant）。
        [Test]
        public void CtxGtZero_MaterialHasClippedKeyword()
        {
            var mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            var white = Texture2D.whiteTexture;
            var m0 = mm.Get(0, white, 0, false);
            var m1 = mm.Get(0, white, 1, false);
            Assert.IsFalse(m0.IsKeywordEnabled("CLIPPED"), "ctx=0: 不裁剪，无 CLIPPED keyword");
            Assert.IsTrue(m1.IsKeywordEnabled("CLIPPED"), "ctx>0: 启用 CLIPPED variant");
        }

        /// SetClipBox 后该 ctx material 的 _ClipBox 被刷新（SetVector 落到缓存实例）—— SetClipBox-after-Get 顺序。
        [Test]
        public void SetClipBox_UpdatesCachedMaterialVector()
        {
            var mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            var white = Texture2D.whiteTexture;
            var m = mm.Get(0, white, 7, false);   // 建 ctx=7 material
            var box = new Vector4(-1.5f, 2.5f, 0.01f, 0.02f);
            mm.SetClipBox(7, box);
            Assert.AreEqual(box, m.GetVector("_ClipBox"), "SetClipBox 应刷新已缓存 material 的 _ClipBox");
        }

        /// SetClipBox-before-Get 顺序：box 先进 dict，Get 建材质时读取。
        [Test]
        public void SetClipBox_BeforeGet_AppliedOnCreation()
        {
            var mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            var white = Texture2D.whiteTexture;
            var box = new Vector4(-2f, 2f, 0.01f, 0.01f);
            mm.SetClipBox(3, box);          // dict 先写
            var m = mm.Get(0, white, 3, false);    // 建材质时应读取 dict
            Assert.AreEqual(box, m.GetVector("_ClipBox"), "首帧：SetClipBox 先于 Get，Get 建材质时带 box");
            Assert.IsTrue(m.IsKeywordEnabled("CLIPPED"));
        }
    }
}
