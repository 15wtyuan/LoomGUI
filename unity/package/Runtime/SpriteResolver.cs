using System;
using System.Collections.Generic;
using UnityEngine;

namespace Yio
{
    /// <summary>
    /// Sprite lookup result from self-drawn atlas. Texture + UV rect (in atlas) + original pixel size.
    /// uvRect uses Unity Rect convention: x=u0, y=v0, width=u1-u0, height=v1-v0.
    /// </summary>
    public struct SpriteLookup
    {
        public Texture2D tex;
        public UnityEngine.Rect uvRect;
        public int origW;
        public int origH;
        public bool found;

        /// <summary>
        /// 命中的图集页键（atlasIdx, page）。null = 非页承载（字体页 / miss / 空键）——
        /// 逐出续命（StampPage）只对页承载的命中生效；字体页在 _fontPages 独立字典，
        /// 结构上不在逐出面。
        /// </summary>
        public (int AtlasIdx, int Page)? PageKey;
    }

    /// <summary>
    /// Font-atlas image_path construction. Must match Rust render::font_atlas_path
    /// (image_path field in blob). Changing the format here requires changing both sides.
    /// </summary>
    public static class FontAtlasPath
    {
        public static string Format(uint page) => $"yio://font-atlas/p{page}";
    }

    /// <summary>
    /// Sprite key to texture + UV rect resolver.
    /// Consumes self-drawn atlas.png + atlas.json (UV table) from the standalone packer.
    /// Drops Unity SpriteAtlas/Sprite dependency entirely — a cross-engine portability win
    /// (Godot/UE load a texture + UV table the same way).
    ///
    /// All atlases are merged into one global sprite table at Init. Page textures are
    /// lazy-loaded via the loadPage delegate. Font atlas pages are registered separately
    /// and take priority in GetSprite lookup.
    /// </summary>
    public sealed class SpriteResolver
    {
        // Merged sprite table: sprite_key → (atlasIdx, page, uvRect, origW, origH).
        Dictionary<string, (int atlasIdx, int page, UnityEngine.Rect uvRect, int origW, int origH)> _sprites;

        /// <summary>页缓存条目：纹理 + 最后使用时刻（Time.unscaledTime 秒）。</summary>
        struct PageEntry
        {
            public Texture2D Tex;
            public float LastUsed;
        }

        // Page texture cache: (atlasIdx, page) → entry. Lazy-loaded via loadPage delegate.
        // 字体页不在此表（_fontPages 独立字典）——逐出只扫本表，字体页结构上天然豁免。
        Dictionary<(int, int), PageEntry> _pageCache;

        // Sweep 受害者收集，成员复用（每帧零分配）。
        readonly List<(int, int)> _evictScratch = new List<(int, int)>();

        /// <summary>
        /// 页纹理逐出宽限期（unscaled 秒）：一页从最后一次被画起计时，超时即 Destroy。
        /// expireAfterAccess 语义（每次使用续命）；≤0 禁用逐出。10s 默认压「刚逐出又要回」
        /// 的重载抖动；秒制而非帧制——30fps 移动端与 144fps 桌面同一语义。
        /// </summary>
        public float PageEvictionGraceSeconds { get; set; } = 10f;

        /// <summary>当前缓存的页纹理数（含未过宽限期的闲置页）。</summary>
        public int PagesAlive => _pageCache?.Count ?? 0;

        /// <summary>会话累计逐出页数（Clear 归零）。验收读数：逐出后重载应 +1 且不再变。</summary>
        public int PagesEvictedTotal { get; private set; }

        // Font atlas pages: path → full-region SpriteLookup. Registered externally by SyncFontAtlas.
        Dictionary<string, SpriteLookup> _fontPages;

        // Miss dedup: warn once per missing key per session.
        HashSet<string> _warned;

        // loadPage delegate: pageFileName (e.g. "ui.png") → Texture2D.
        Func<string, Texture2D> _loadPage;

        // Atlas page filenames indexed by (atlasIdx, page).
        List<List<string>> _atlasPages;

        /// <summary>
        /// Initialize from atlas manifests. Merges ALL atlases' sprites into one global table.
        /// loadPage(pageFileName) lazily loads a page texture (e.g. "ui.png").
        /// atlases=null → empty (safe to call GetSprite — all miss).
        /// </summary>
        public void Init(List<AtlasManifest> atlases, Func<string, Texture2D> loadPage)
        {
            _sprites = new Dictionary<string, (int, int, UnityEngine.Rect, int, int)>();
            _pageCache = new Dictionary<(int, int), PageEntry>();
            _fontPages = new Dictionary<string, SpriteLookup>();
            _warned = new HashSet<string>();
            _loadPage = loadPage;
            _atlasPages = new List<List<string>>();

            if (atlases == null) return;

            for (int atlasIdx = 0; atlasIdx < atlases.Count; atlasIdx++)
            {
                var atlas = atlases[atlasIdx];
                if (atlas == null) { _atlasPages.Add(new List<string>()); continue; }
                _atlasPages.Add(atlas.pages ?? new List<string>());

                if (atlas.sprites == null) continue;
                foreach (var kv in atlas.sprites)
                {
                    var entry = kv.Value;
                    var uv = entry.uv;
                    if (uv == null || uv.Length < 4) continue;
                    if (entry.orig == null || entry.orig.Length < 2) continue;
                    // atlas.json 的 uv 是像素左上原点（v0=顶，打包器按 image crate 约定算）；
                    // Unity 纹理采样 v=0 在底，故翻转 v：y = 1 - v1（atlas 底对应 Unity 底）。
                    var uvRect = new UnityEngine.Rect(uv[0], 1f - uv[3], uv[2] - uv[0], uv[3] - uv[1]);
                    _sprites[kv.Key] = (atlasIdx, entry.page, uvRect, entry.orig[0], entry.orig[1]);
                }
            }
        }

        /// <summary>
        /// Look up a sprite by its workspace-relative key.
        /// Returns SpriteLookup with found=false on miss (caller fallback).
        /// Empty/null key returns found=false without warning.
        /// </summary>
        public SpriteLookup GetSprite(string key)
        {
            if (string.IsNullOrEmpty(key))
                return new SpriteLookup { found = false };

            // Font atlas pages take priority — check before sprite table.
            if (_fontPages != null && _fontPages.TryGetValue(key, out var fontLookup))
                return fontLookup;

            if (_sprites == null || !_sprites.TryGetValue(key, out var entry))
            {
                if (_warned != null && _warned.Add(key))
                    Debug.LogWarning($"[SpriteResolver] sprite not found: '{key}'");
                return new SpriteLookup { found = false };
            }

            Texture2D tex = GetOrLoadPage(entry.atlasIdx, entry.page);
            if (tex == null)
            {
                if (_warned != null && _warned.Add(key))
                    Debug.LogWarning($"[SpriteResolver] page tex load fail: atlas[{entry.atlasIdx}] p{entry.page}, key='{key}'");
                return new SpriteLookup { found = false };
            }

            return new SpriteLookup
            {
                tex = tex,
                uvRect = entry.uvRect,
                origW = entry.origW,
                origH = entry.origH,
                found = true,
                PageKey = (entry.atlasIdx, entry.page)
            };
        }

        /// <summary>
        /// Register a font atlas page. Text mesh image_path="yio://font-atlas/p{n}"
        /// hits this cache via GetSprite, returning a full-region (0,0,1,1) SpriteLookup.
        /// Re-registering the same path replaces the old entry (font atlas pages are immutable
        /// per-session; old Texture2D is GC'd).
        /// </summary>
        public void RegisterFontAtlasPage(string path, Texture2D tex)
        {
            if (tex == null) return;
            if (_fontPages == null)
                _fontPages = new Dictionary<string, SpriteLookup>();
            _fontPages[path] = new SpriteLookup
            {
                tex = tex,
                uvRect = new UnityEngine.Rect(0, 0, 1, 1),
                origW = tex.width,
                origH = tex.height,
                found = true
            };
        }

        public void Clear()
        {
            _sprites?.Clear();
            _pageCache?.Clear();
            _fontPages?.Clear();
            _warned?.Clear();
            _evictScratch.Clear();
            PagesEvictedTotal = 0;
        }

        /// <summary>
        /// 逐出闲置页（MirrorPool.Sync 每帧驱动）。判据 = 「这帧没有画面在画它」连续超过
        /// PageEvictionGraceSeconds——证据有两路：① GetSprite 的盖章（变更帧的加载/换图），
        /// ② StampPage 的镜像侧盖章（Skip 行不进 lean 段、变更帧零 GetSprite——闲置静态页
        /// 靠 MirrorPool 每帧代 active GO 盖章续命，缺它则静态页的图集页在宽限期满后被
        /// 销毁而 mesh 材质仍引用已销毁纹理）。语义引用不看：display:none / 滚出视口的节点
        /// 不盖章，其页到期即逐出，重新可见时经 GetOrLoadPage 现载（重激活同帧经 lean 行
        /// UpdateHeader 重绑纹理，不会引用已销毁纹理）。Destroy 是释放级廉价操作（非托管
        /// GC 停顿），错峰死亡随各页最后使用时刻自然分布，无批量回收峰值。仅 PlayMode：
        /// EditMode 无长会话内存压力，且 Object.Destroy 在编辑态非法。
        /// </summary>
        public void Sweep()
        {
            if (!Application.isPlaying) return;
            if (_pageCache == null || _pageCache.Count == 0) return;
            float grace = PageEvictionGraceSeconds;
            if (grace <= 0f) return;

            float now = Time.unscaledTime;
            foreach (var kv in _pageCache)
                if (now - kv.Value.LastUsed > grace)
                    _evictScratch.Add(kv.Key);
            for (int i = 0; i < _evictScratch.Count; i++)
            {
                PageEntry e = _pageCache[_evictScratch[i]];
                _pageCache.Remove(_evictScratch[i]);
                if (e.Tex != null)
                {
                    UnityEngine.Object.Destroy(e.Tex);
                    PagesEvictedTotal++;
                }
            }
            _evictScratch.Clear();
        }

        /// <summary>
        /// 镜像侧续命（MirrorPool.Sync 每帧对每个 active RenderObj 的绑定页调用）：
        /// 页在缓存中则刷新最后使用时刻；不在缓存 = 已逐出/未加载，无需动作——下次
        /// 变更帧的 GetSprite 会现载。缺缓存条目时零写入，闲置池迭代零分配。
        /// </summary>
        public void StampPage((int AtlasIdx, int Page) key)
        {
            if (_pageCache == null) return;
            if (_pageCache.TryGetValue(key, out var e))
            {
                e.LastUsed = Time.unscaledTime;
                _pageCache[key] = e;    // struct：改字段须写回
            }
        }

        Texture2D GetOrLoadPage(int atlasIdx, int page)
        {
            var key = (atlasIdx, page);
            if (_pageCache == null) return null;
            // 命中即盖章（expireAfterAccess 的续命点）：MirrorPool lean 段每帧对每个可见
            // mesh 节点走到这里——盖到的页 = 本帧有画面在画，逐出判据的证据源。
            if (_pageCache.TryGetValue(key, out var cached))
            {
                cached.LastUsed = Time.unscaledTime;
                _pageCache[key] = cached;   // struct：改字段须写回
                return cached.Tex;
            }

            if (_loadPage == null) return null;
            if (_atlasPages == null || atlasIdx >= _atlasPages.Count) return null;
            var pages = _atlasPages[atlasIdx];
            if (page < 0 || page >= pages.Count) return null;

            string fileName = pages[page];
            Texture2D tex = _loadPage(fileName);
            if (tex != null) _pageCache[key] = new PageEntry { Tex = tex, LastUsed = Time.unscaledTime };
            return tex;
        }
    }
}
