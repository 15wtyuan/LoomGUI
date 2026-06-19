using System.Collections.Generic;
using UnityEngine;

namespace LoomGUI
{
    /// DrawState 缓存（§8.4，照 fgui MaterialManager）。
    /// key = (program, texture, mask_context)。同 key 复用 Material 实例。
    /// tint×alpha 走顶点色（不在 key 里）；clip_box 进 mask_context 专属 Material 的 _ClipBox uniform。
    public sealed class MaterialManager
    {
        readonly Shader _shader;
        readonly Dictionary<Key, Material> _cache = new();
        // Phase 1：mask_context 恒 0，不 enable CLIPPED；Phase 2 rect mask 在此按 mask_context 设 _ClipBox + EnableKeyword。
        readonly Dictionary<uint, Vector4> _clipBoxByCtx = new();

        public MaterialManager(Shader shader) { _shader = shader; }

        public Material Get(int program, Texture texture, uint maskContext)
        {
            var key = new Key(program, texture ? texture.GetEntityId() : 0, maskContext);
            if (!_cache.TryGetValue(key, out var mat))
            {
                mat = new Material(_shader);
                mat.mainTexture = texture;
                mat.SetFloat("_SrcFactor", 5f);   // SrcAlpha
                mat.SetFloat("_DstFactor", 10f);  // OneMinusSrcAlpha
                if (_clipBoxByCtx.TryGetValue(maskContext, out var cb))
                {
                    mat.SetVector("_ClipBox", cb);
                    mat.EnableKeyword("CLIPPED");
                }
                _cache[key] = mat;
            }
            return mat;
        }

        /// Phase 2 rect mask 用：注册某 mask_context 的 clip_box。
        public void SetClipBox(uint maskContext, Vector4 clipBox) => _clipBoxByCtx[maskContext] = clipBox;

        public void Clear()
        {
            foreach (var kv in _cache) Object.Destroy(kv.Value);
            _cache.Clear();
        }

        readonly struct Key
        {
            readonly int _program, _tex;
            readonly uint _ctx;
            public Key(int p, int t, uint c) { _program = p; _tex = t; _ctx = c; }
            public override int GetHashCode() => System.HashCode.Combine(_program, _tex, (int)_ctx);
            public override bool Equals(object o) => o is Key k && k._program == _program && k._tex == _tex && k._ctx == _ctx;
        }
    }
}
