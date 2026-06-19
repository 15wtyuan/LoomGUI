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
            var a = mm.Get(program: 0, white, maskContext: 0);
            var b = mm.Get(program: 0, white, maskContext: 0);
            Assert.AreSame(a, b);
        }

        [Test]
        public void DifferentMaskContextReturnsDifferentMaterial()
        {
            var mm = new MaterialManager(Shader.Find("LoomGUI/Unlit"));
            var white = Texture2D.whiteTexture;
            var a = mm.Get(0, white, 0);
            var b = mm.Get(0, white, 1);
            Assert.AreNotSame(a, b);
        }
    }
}
