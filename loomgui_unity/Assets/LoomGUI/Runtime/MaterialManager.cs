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

        public Material Get(int program, Texture texture, uint maskContext, bool matrixFlag)
        {
            var key = new Key(program, texture, maskContext, matrixFlag);
            if (!_cache.TryGetValue(key, out var mat))
            {
                mat = new Material(_shader);
                mat.mainTexture = texture;
                mat.SetFloat("_SrcFactor", 5f);   // SrcAlpha
                mat.SetFloat("_DstFactor", 10f);  // OneMinusSrcAlpha
                if (maskContext > 0u)
                {
                    // §4.4：ctx>0 → CLIPPED 变体（multi_compile _ CLIPPED，shader 端 discard）。
                    // mask_context 进 key，每 ctx 独立 Material 实例，keyword 设该实例。
                    mat.EnableKeyword("CLIPPED");
                    // 首帧路径：MirrorPool 先 SetClipBox（box 进 _clipBoxByCtx）再 Get；
                    // 新建 Material 时从此 dict 读 box。后续帧材质已缓存，SetClipBox 的 SetVector 分支刷新。
                    if (_clipBoxByCtx.TryGetValue(maskContext, out var cb))
                        mat.SetVector("_ClipBox", cb);
                }
                if (matrixFlag) mat.EnableKeyword("OBJECT_MATRIX");
                _cache[key] = mat;
            }
            return mat;
        }

        /// §4.4：注册某 mask_context 的 _ClipBox。先写 _clipBoxByCtx（新建 Material 时 Get 会带上），
        /// 再把已缓存 Material 实例的 _ClipBox 同步刷新（每 ctx 一实例，fgui group=clipId 语义）。
        /// 两路都覆盖：SetClipBox 既可在 Get 前（首帧：box 进 dict，Get 建材质时读取）也可在 Get 后
        /// （后续帧：材质已存，直接 SetVector 刷新）。故调用顺序对 MirrorPool 不构成约束。
        public void SetClipBox(uint maskContext, Vector4 clipBox)
        {
            _clipBoxByCtx[maskContext] = clipBox;
            foreach (var kv in _cache)
                if (kv.Key.Ctx == maskContext) kv.Value.SetVector("_ClipBox", clipBox);
        }

        public void Clear()
        {
            foreach (var kv in _cache)
            {
                if (Application.isPlaying) Object.Destroy(kv.Value);
                else Object.DestroyImmediate(kv.Value);   // [ExecuteAlways] 编辑器预览走 Edit mode
            }
            _cache.Clear();
        }

        // key 持 Texture 引用（Unity 对象同一性），避开 Unity 6.5 废弃的 GetInstanceID/GetEntityId/EntityId。
        // 材质与纹理同生命周期，缓存随纹理存活正确；v1b 纹理释放时配 eviction。
        readonly struct Key
        {
            readonly int _program;
            readonly Texture _tex;
            readonly uint _ctx;
            readonly bool _matrix;
            public Key(int p, Texture t, uint c, bool m) { _program = p; _tex = t; _ctx = c; _matrix = m; }
            public uint Ctx => _ctx;   // SetClipBox 按 ctx 反查已缓存 material（独立于 program/tex）。
            public override int GetHashCode() => System.HashCode.Combine(_program, _tex, (int)_ctx, _matrix);
            public override bool Equals(object o) => o is Key k
                && k._program == _program
                && k._tex == _tex
                && k._ctx == _ctx
                && k._matrix == _matrix;
        }
    }
}
