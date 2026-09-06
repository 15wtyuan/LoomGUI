using System;
using System.Buffers;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using Yio.Bindings;
using UnityEngine;

namespace Yio
{
    /// <summary>
    /// Unity 引擎后端实现：持 MirrorPool / MaterialManager / NativeHostManager / SpriteResolver /
    /// YioInputCollector。<see cref="YioHost"/> 通过
    /// <see cref="YioBackend"/> 契约驱动：每帧先 <see cref="CollectInput"/> 再 <see cref="SyncFrame"/>
    /// （borrow_frame 已由 YioHost 完成，ptr+len 传入避免二次 borrow）。
    ///
    /// NativeHost（GameObject 绑定 3D 模型，<see cref="NativeHostManager"/>）是 Unity 专属，
    /// 不进 <see cref="YioBackend"/> 通用契约，作额外属性 <see cref="NativeHost"/> 暴露给 YioHost。
    ///
    /// 资源回收：本类实现 <see cref="IDisposable"/>——Driver.OnDestroy 调 <see cref="Dispose"/>
    /// 清理 MirrorPool GO / MaterialManager 材质 / NativeHostManager wrapper / SpriteResolver 缓存 /
    /// ArrayPool frame buffer。<see cref="YioHost.Dispose"/> 只释放 stage 句柄（引擎中立），
    /// 引擎资源归本类。
    /// </summary>
    public sealed unsafe class UnityYioBackend : YioBackend, IDisposable
    {
        readonly MirrorPool _pool = new();
        MaterialManager _mm;                 // ctgfx 程序化 material 缓存（Shader Find 后注入）
        readonly NativeHostManager _nhm = new();
        internal SpriteResolver _sprites = new();  // YioHost InitSprites/SetImageSizes 资源注册用
        YioInputCollector _inputCollector;  // Driver Awake 注入（指针/键盘/滚轮采集）
        Transform _renderRoot;               // MirrorPool 镜像 GO + NativeHost container 挂此 root
        byte[] _frameBuf;                    // ArrayPool 租用（搬自 YioStage.Tick 的复用语义）
        int _lastFrameLen;                   // 诊断：上一帧 blob 实际字节数（_frameBuf 可能 Rent 超长）
        uint _lastFfiPanicCount;             // Rust FFI guard 兜底计数上次采样（变化即有 panic 被吞）
        readonly YioResourceHost _resourceHost; // 共享资源宿主（null = 单 Stage 自建宿主，atlas 走本类 per-stage 路径）

        /// <param name="mm">由 Driver 构造并注入（Shader.Find("Yio/Unlit") 后建）。</param>
        /// <param name="resourceHost">共享资源宿主（多 Stage 共享字体/atlas）；null = 单 Stage 兼容路径。</param>
        public UnityYioBackend(MaterialManager mm, YioResourceHost resourceHost = null)
        {
            _mm = mm;
            _resourceHost = resourceHost;
        }

        /// <summary>A4 输入路由门：Driver 每帧下发（hub 独占路由结果）。false = CollectInput no-op。</summary>
        public bool InputEnabled = true;

        /// <summary>A4 排序基址下发：MirrorPool 与 NativeHost 的 sortingOrder 同源偏移。</summary>
        internal void SetSortBase(int sortBase)
        {
            _pool.SortBase = sortBase;
            _nhm.SortBase = sortBase;
        }

        /// <summary>C8 挂载容器登记（Driver BindWorldMount → MirrorPool 路由表）。</summary>
        internal void SetMountContainer(ulong slot, Transform container) =>
            _pool.SetMountContainer(slot, container);

        /// <summary>C8 解除挂载容器（镜像 GO 先行挂回屏幕 root，容器销毁归 Driver）。</summary>
        internal void ClearMountContainer(ulong slot) =>
            _pool.ClearMountContainer(slot, _renderRoot);

        /// <summary>
        /// Driver Awake 注入：渲染根（MirrorPool / NativeHost 镜像 GO 挂此 root）+ 输入采集器。
        /// 必须在第一次 <see cref="SyncFrame"/> 前调——SyncFrame 读 _renderRoot，null 时跳过镜像。
        /// NativeHostManager.Init 也在此调用方（Driver）建 container；本 backend 不重复建。
        /// </summary>
        public void SetRuntimeRoot(Transform root, YioInputCollector input)
        {
            _renderRoot = root;
            _inputCollector = input;
        }

        /// <summary>
        /// NativeHost 绑定点（Unity 专属，不进 YioBackend 通用契约）。
        /// internal——<see cref="NativeHostManager"/> 自身是 internal sealed，YioHost 同程序集可见。
        /// </summary>
        internal NativeHostManager NativeHost => _nhm;

        /// <summary>
        /// 图集页解析器（Driver/消费侧诊断读数用：PagesAlive / PagesEvictedTotal /
        /// PageEvictionGraceSeconds——#62 页纹理逐出的观察面）。只读用途；写入走 InitSprites。
        /// </summary>
        public SpriteResolver Sprites => _sprites;

        /// <summary>
        /// SpriteResolver 初始化：传入所有 atlas manifest + 页纹理懒加载委托。
        /// Driver.Awake 后调：ParseAtlas 解析每个 atlas.json → <see cref="AtlasManifest"/>，传入此方法。
        /// loadPage(pageFileName) 按需加载页 PNG（Driver 决定走 Resources/AB/Addressables）。
        /// loadPage=null 则 GetSprite 全 miss（调用方 fallback）。
        ///
        /// Unity 特定资源 IO（Texture2D）——不进 <see cref="YioHost"/> 引擎无关层。
        /// </summary>
        public void InitSprites(List<AtlasManifest> atlases, Func<string, Texture2D> loadPage)
        {
            _sprites?.Init(atlases, loadPage);
        }

        /// <summary>
        /// 采集 Unity 输入（指针/键盘/滚轮）→ set_input 系 FFI（引擎中立，由 YioInputCollector 内部调）。
        /// DesignSize 由 YioInputCollector 实例携带（Driver Awake 注入）。
        /// </summary>
        public override void CollectInput(IntPtr stage)
        {
            // A4 多 Stage 输入隔离：非本帧路由所有者的 Driver 整体跳过采集
            // （hub 按层序 Pick 独占路由；单 Driver 恒 true = 零行为变化）。
            if (stage == IntPtr.Zero || _inputCollector == null || !InputEnabled) return;
            _inputCollector.Collect(stage);
            _inputCollector.CollectKeys(stage);
            // CollectComposition 须在 CollectText 前：先设/清 IME 预编辑串，CollectText 再
            // insert 组字完成的结果字符（inputString）。
            _inputCollector.CollectComposition(stage);
            _inputCollector.CollectText(stage);
            YioInputCollector.CollectWheel(stage, _inputCollector);
        }

        /// <summary>
        /// 消费 borrow_frame blob → ArrayPool 复制 → <see cref="SyncFontAtlas"/>（脏页上传）+
        /// <see cref="MirrorPool.Sync"/>（RenderNode 镜像）+ <see cref="NativeHostManager.Sync"/>（3D 模型绑定）。
        /// </summary>
        public override void SyncFrame(IntPtr stage, IntPtr framePtr, int frameLen)
        {
            WarnOnNativePanic();
            if (framePtr == IntPtr.Zero || frameLen <= 0 || _renderRoot == null) return;
            StageHandle* h = (StageHandle*)stage.ToPointer();

            // frame buffer（ArrayPool 复用）。Rent 返 ≥len，只 copy/解析 len 字节。
            if (_frameBuf == null || _frameBuf.Length < frameLen)
            {
                if (_frameBuf != null) ArrayPool<byte>.Shared.Return(_frameBuf);
                _frameBuf = ArrayPool<byte>.Shared.Rent(frameLen);
            }
            Marshal.Copy(framePtr, _frameBuf, 0, frameLen);
            _lastFrameLen = frameLen;
            var blob = new FrameBlob(_frameBuf);

            SyncFontAtlas(h);
            // v10：不再传字体表（核心自产 atlas，后端不再光栅化文本）。
            _pool.Sync(blob, _renderRoot, _mm, _sprites, Texture2D.whiteTexture);
            _nhm.Sync(h);
        }

        /// <summary>
        /// Rust FFI panic 兜底计数轮询。native 侧全部导出包 catch_unwind（panic 穿越 extern "C"
        /// 会 abort 宿主进程，不可接受），被吞的 panic 意味着该次调用失败、Stage 可能半修改——
        /// 必须可见。计数变化即 LogError；panic 位置/消息由 Rust 默认 panic hook 打到 native 日志。
        /// </summary>
        void WarnOnNativePanic()
        {
            uint count = Native.yio_ffi_panic_count();
            if (count == _lastFfiPanicCount) return;
            Debug.LogError($"[Yio] native panic caught by FFI guard (total {count}); frame state may be inconsistent — check native log for panic origin");
            _lastFfiPanicCount = count;
        }

        /// <summary>
        /// 拉取核心字体 atlas 脏页 → 上传 R8 Texture2D → Sprite 包装 → 注册进 SpriteResolver。
        /// SyncFrame 内 <see cref="MirrorPool.Sync"/> 前调——本帧渲染节点包含 text Mesh image_path，
        /// 先注册 atlas Sprite 使 text 节点的 image_path 命中 GetSprite 缓存。
        ///
        /// 共享宿主（多 Stage）：走 <see cref="YioResourceHost.SyncAtlas"/> 单点拉取
        /// （脏页 clear 是全局的，多 driver 各拉各清会让后拉者永远缺新字形页）。
        /// 单 Stage：本类 per-stage 路径（双调法取页数据：先探 buf_len=0 返所需字节数
        /// → 分配 buf → 再调填 w/h/bytes。页面通常 512×512=256KB，独立 ArrayPool 缓冲）。
        /// </summary>
        unsafe void SyncFontAtlas(StageHandle* h)
        {
            if (_resourceHost != null)
            {
                _resourceHost.SyncAtlas(_sprites);
                return;
            }
            // 探脏页（通常 ≤8 页；单字体极少超 16）。
            const int MAX_DIRTY = 16;
            uint* dirtyPtr = stackalloc uint[MAX_DIRTY];
            int n = (int)Native.yio_stage_font_atlas_dirty_pages(h, dirtyPtr, (nuint)MAX_DIRTY);
            if (n <= 0) return;
            if (n > MAX_DIRTY)
            {
                Debug.LogWarning($"[UnityYioBackend] font atlas dirty pages ({n}) exceed MAX_DIRTY ({MAX_DIRTY}); skipping extras");
                n = MAX_DIRTY;
            }

            for (int i = 0; i < n; i++)
            {
                uint page = dirtyPtr[i];
                // 此处外层参数已名 h（StageHandle*），重名局 page height 为 ph 避冲突。
                uint w = 0, ph = 0;
                // 探所需字节数（buf_len=0, out_buf=null → 返 needed 不写 w/h/pixels）。
                int needed = (int)Native.yio_stage_font_atlas_page(h, page, &w, &ph, null, (nuint)0);
                if (needed <= 0) continue;

                byte[] buf = ArrayPool<byte>.Shared.Rent(needed);
                try
                {
                    fixed (byte* pBuf = buf)
                    {
                        int got = (int)Native.yio_stage_font_atlas_page(h, page, &w, &ph, pBuf, (nuint)needed);
                        if (got != needed) continue;
                    }
                    // R8 必须用 linear=true：distance 存在 .r，默认 sRGB 采样会被硬件 sRGB→Linear 解码
                    // 把 d 压低（inside 0.59→0.30）→ faceAlpha 算成 0 → 字消失。linear=true 直读 raw byte。
                    var tex = new Texture2D((int)w, (int)ph, TextureFormat.R8, false, true);
                    fixed (byte* p = buf) { tex.LoadRawTextureData((IntPtr)p, needed); }
                    tex.Apply(false, true);
                    // atlas 是 Stage 级单一共享实例（所有字体字形混在同一 page），路径只以 page 为键——
                    // 不含 font_id（font_id 只作 GlyphKey 区分字形槽位，不进 path）。与 render 侧
                    // build_text_mesh 合成的 yio://font-atlas/p{n} 对齐。
                    string path = FontAtlasPath.Format(page);
                    _sprites.RegisterFontAtlasPage(path, tex);
                }
                finally { ArrayPool<byte>.Shared.Return(buf); }
            }
            Native.yio_stage_font_atlas_clear_dirty(h);
        }

        /// <summary>诊断：dump 当前 MirrorPool GO 状态（Unity 渲染视角）。</summary>
        public string DumpMirrorState() => _pool?.DumpState() ?? "(pool null)";

        /// <summary>
        /// 诊断：dump 当前 blob 每个渲染节点（core 视角）：node_id + world_matrix tx/ty +
        /// mesh vert bbox。对照 DumpMirrorState 看两层是否一致。仅 dump 非 SKIP 节点。
        /// </summary>
        public string DumpBlobState()
        {
            var sb = new System.Text.StringBuilder();
            if (_frameBuf == null || _lastFrameLen <= 0) { sb.AppendLine("(no frame yet)"); return sb.ToString(); }
            // FrameBlob 只解析 _frameBuf 前 _lastFrameLen 字节；Rent 超长部分是垃圾。
            var blob = new FrameBlob(_frameBuf);
            if (!blob.IsValid) { sb.AppendLine($"(blob invalid magic/version, len={_lastFrameLen})"); return sb.ToString(); }
            sb.AppendLine($"[Blob] nodes={blob.NodeCount} (lean={blob.LeanCount} skip={blob.SkipCount})");
            sb.AppendLine("  i    nodeId   mask  pure  Mtx    Mty    program  meshBBox");
            for (int i = 0; i < blob.LeanCount; i++)
            {
                if (!blob.Visible(i)) continue;
                // 只 dump 有 mesh 的节点（PayloadKind=1），减少噪音
                if (blob.PayloadKind(i) != 1) continue;
                float mtx = blob.Mtx(i), mty = blob.Mty(i);
                bool pure = blob.IsPureTranslation(i);
                byte level = blob.ChangeLevel(i);  // 1=Header 2=Full（Skip 在 skip 段）
                uint mask = blob.MaskContext(i);
                // 读 mesh bbox：仅 Full 节点 mesh_off/len>0（Header 占位 0，读 arena 开头是垃圾）
                string bbox;
                uint meshLen = blob.ReadMeshLenRaw(i);
                if (meshLen == 0)
                {
                    bbox = $"(no-mesh level={level})";
                }
                else
                {
                    var seg = blob.ReadMesh(i);
                    float minx = float.MaxValue, miny = float.MaxValue, maxx = float.MinValue, maxy = float.MinValue;
                    for (int v = 0; v < seg.Verts.Length; v++)
                    {
                        minx = Math.Min(minx, seg.Verts[v].x); miny = Math.Min(miny, seg.Verts[v].y);
                        maxx = Math.Max(maxx, seg.Verts[v].x); maxy = Math.Max(maxy, seg.Verts[v].y);
                    }
                    bbox = seg.Verts.Length > 0 ? $"({minx:F0},{miny:F0})~({maxx:F0},{maxy:F0})" : "(empty)";
                }
                sb.AppendLine($"  {i,3} {blob.NodeId(i),8} m={mask,3} {(pure?"P":"T")} ({mtx,6:F0},{mty,6:F0}) prog={blob.Program(i)} lv={level} mb={bbox}");
            }
            // skip 段摘要（v15：Skip 行 + parked keepalive）。
            int parkedCount = 0;
            for (int s = 0; s < blob.SkipCount; s++) if (blob.SkipParked(s)) parkedCount++;
            sb.AppendLine($"  [skip segment] count={blob.SkipCount} (parked={parkedCount})");
            // clip 表（#52 多 entry 布局：92B entry + poly arena）：per-entry 打印
            // ctx/flags/inv_frame 平移/几何 kind——shapeKind 1=circle 2=polygon，
            // rectKind 1=直角 2=圆角。取证 var 换形滞后类问题：对照 [MirrorPool] 段
            // 的材质 kind 回读，两侧不一致即定位到刷新链路。
            sb.AppendLine($"  [clip table] count={blob.ClipCount}");
            for (int c = 0; c < blob.ClipCount; c++)
            {
                int p = blob.ClipTableOffPub + 4 + c * 92;
                uint ctx = blob.ReadU32Public(p);
                uint flags = blob.ReadU32Public(p + 4);
                float tx = blob.ReadF32Public(p + 24), ty = blob.ReadF32Public(p + 28);
                float rw = blob.ReadF32Public(p + 32), rh = blob.ReadF32Public(p + 36);
                int polyCount = (int)blob.ReadU32Public(p + 84);
                int shapeKind = (int)((flags >> 8) & 0xFF);
                bool hasRect = (flags & 1) != 0, hasRadii = (flags & 2) != 0, hasShape = (flags & 4) != 0;
                string kind = hasShape
                    ? (shapeKind == 1 ? $"circle(r={blob.ReadF32Public(p + 80):F0})" : $"poly({polyCount})")
                    : (hasRect ? (hasRadii ? "rounded" : "rect") : "none");
                sb.AppendLine($"    ctx={ctx} t=({tx:F0},{ty:F0}) box=({rw:F0}x{rh:F0}) {kind}");
            }
            return sb.ToString();
        }

        /// <summary>
        /// 释放 Unity 引擎资源：MirrorPool 镜像 GO + NativeHostManager wrapper + MaterialManager 材质 +
        /// SpriteResolver 缓存 + ArrayPool frame buffer。<see cref="YioHost"/>.Dispose 不递归——
        /// 引擎资源归本类。Driver.OnDestroy 先 host.Dispose（释放 stage 句柄）再 backend.Dispose。
        /// SpriteResolver 持 lazy-loaded 页 Texture2D 缓存，Clear 清表但不 Dispose 页纹理
        /// （归 caller / 构建后端拥有其生命周期）。
        /// </summary>
        public void Dispose()
        {
            _pool?.Clear();
            _nhm?.Clear();
            _mm?.Clear();
            _sprites?.Clear();
            if (_frameBuf != null)
            {
                ArrayPool<byte>.Shared.Return(_frameBuf);
                _frameBuf = null;
            }
        }
    }
}
