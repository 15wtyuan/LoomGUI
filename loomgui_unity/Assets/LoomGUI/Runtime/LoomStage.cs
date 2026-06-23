using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using LoomGUI.Bindings;
using UnityEngine;

namespace LoomGUI
{
    /// <summary>
    /// v1a Phase 1 集成入口（§14 驱动）：把 Rust Stage（tick→borrow_frame→blob）接到 Unity
    /// MirrorPool 渲染。挂场景即跑：Awake 建 stage+load_html+配置根/相机；LateUpdate 每帧
    /// tick→borrow→Marshal.Copy→FrameBlob→MirrorPool.Sync。
    ///
    /// 设计坐标系：origin 左上、y-down，单位 design px（_designSize）。根 transform 一次做
    /// MatchWidthOrHeight 缩放 + y-flip（localScale=(sf,-sf,sf)）+ 平移到屏幕左上原点。
    /// UI 相机正交、cullingMask=LoomUI(layer 6)、orthoSize=Screen.height/2，独立于根（不被
    /// 根的负 scale 影响）。shader Cull Off 吸收翻转的 winding。
    /// </summary>
    [ExecuteAlways]
    public sealed unsafe class LoomStage : MonoBehaviour
    {
        // v0 解析器不读 inline style=（见 layout/mod.rs 注释），用 class + 独立 CSS。
        // 默认场景：一个 200×100 红块（视觉最小验证）。多节点版见 _css 注释。
        [SerializeField] string _html = "<div class=\"b\"></div>";
        [SerializeField] string _css =
            ".b{width:200px;height:100px;background-color:#ff0000;}";
        [SerializeField] Vector2 _designSize = new(1080, 1920);
        [SerializeField] Camera _uiCamera;
        // Unity 动态字体（§4.3）：与 Rust measure 的同一份 DejaVuSans.ttf（Assets/LoomGUI/Fonts/）。
        // Inspector 指定为主路径；EditMode 测试 / 未配场景用 AssetDatabase 兜底（见 EnsureFont）。
        [SerializeField] Font _font;

        // §9.4 v1b.5：Stage 读取的 ttf 文件名（StreamingAssets 下，喂 Rust measure）。
        // 默认 "DejaVuSans.ttf" 不破坏现有场景；CJK sample 配 "wqy-microhei.ttc"。
        // 须与 _font（Unity 光栅用）是同一份 ttf（§4.3 跨平台一致性）。
        [SerializeField] string _fontFile = "DejaVuSans.ttf";

        // §13 v1b.1：从二进制包加载（true）vs inline _html/_css（false，默认保现有行为）。
        // true 时从 StreamingAssets/_pkgFile 读 .pkg.bin → loomgui_stage_load_package。
        [SerializeField] bool _usePackage;
        [SerializeField] string _pkgFile = "loom_atlas.pkg.bin";

        // §4.5 500 节点静态压测 fixture：勾选 → Awake 覆盖 _html/_css 为程序生成的 ~500 节点
        // （嵌套 flex column + 每行 colored div + text label，覆盖 mesh + text 双路径）。
        // 默认 false（保持 v1a 单红块场景不变）。PlayMode 肉眼验无卡顿（v1a §9.3 便宜帧）。
        [SerializeField] bool _stress500;
        // OnGUI 左上角 FPS 读数（1/Time.smoothDeltaTime + pool 节点数）。stress500 或本开关任一为真即显。
        [SerializeField] bool _showFps;

        // csbindgen 生成的 Native 用类型化指针 StageHandle*（非 IntPtr）；
        // 借出长度参数是 nuint*（非 ulong*）。故本类标 unsafe 并持 StageHandle*。
        StageHandle* _stage;
        MaterialManager _mm;
        MirrorPool _pool;
        byte[] _frameBuf;
        // v1b.3：tex_id → Texture2D（LoadAtlas 填；Sync 按 blob.TexId 查此绑 atlas 纹理）。
        // 同 atlas 的多个 sprite 共享同一 Texture2D（atlas.png）→ MaterialManager key 命中同实例 → batchable。
        readonly Dictionary<uint, Texture2D> _texMap = new();

        // v1c.1：输入采集 + 事件派发（Inspector 指定；为 null 时跳过输入/事件路径）。
        // _inputCollector 通常与本 MonoBehaviour 同 GO（Awake 时 GetComponent 兜底）。
        [SerializeField] LoomInputCollector _inputCollector;
        // LoomEventHandler 非 UnityEngine.Object（纯 C# class）——不能 SerializeField 持有
        // （Unity 不序列化非 Object 引用字段为资产链接）。改为 Awake new + 外部 AddListener 注册。
        readonly LoomEventHandler _eventHandler = new();

        // 上一帧 Screen 尺寸，检测 resize 重配根/相机。
        int _lastScreenW = -1, _lastScreenH = -1;

        const int LoomUILayer = 6;

        /// v1c.1：游戏侧通过此属性注册 listener（AddListener/RemoveListener），例如
        /// stage.EventHandler.AddListener(nodeId, EventType.Click, OnBtnClick)。
        public LoomEventHandler EventHandler => _eventHandler;

        /// v1c.3：UI 挡住时游戏不响应点击（§10.6）。= 任一活跃槽（鼠标 + 触摸）命中非根节点。
        /// 游戏侧每帧/点击时查此 bool 决定是否消费输入（true → 游戏不响应）。
        public bool IsPointerOnUI()
        {
            if (_stage == null) return false;
            return Native.loomgui_stage_is_pointer_on_ui(_stage);
        }

        void Awake()
        {
            // ExecuteAlways：EditMode/Play 反复 Awake + domain reload 会让上一轮的 loom_node 镜像 GO
            // （root 的子）成孤儿残留——旧 _pool 引用已丢、Clear 不到。开局先清 root 下所有 loom_node
            // 子 GO，防累积泄漏。UI 相机是独立 GO（SetParent(null)），非 root 子，不受影响。
            for (int c = transform.childCount - 1; c >= 0; c--)
            {
                var child = transform.GetChild(c);
                if (child.name == "loom_node")
                    DestroyImmediate(child.gameObject);
            }

            // Stage::new 需字体路径（即使纯色块场景也要加载用于 measure）。
            // Application.streamingAssetsPath：editor 与 player 都可用（editor 返回 Assets/StreamingAssets）。
            string fontPath = System.IO.Path.Combine(Application.streamingAssetsPath, _fontFile);
            byte[] fpBytes = Encoding.UTF8.GetBytes(fontPath);

            fixed (byte* fp = fpBytes)
            {
                _stage = Native.loomgui_stage_new(fp, (nuint)fpBytes.Length, _designSize.x, _designSize.y);
            }
            if (_stage == null)
            {
                Debug.LogError("[LoomStage] loomgui_stage_new 失败（字体路径/不可达？）");
                return;
            }

            // v1c.2-T5：_stage 创建后即 SetHandle（handler.node_parent FFI 需 StageHandle*）。
            // 每次 load 成功后还会再调（清 _parentCache——新 scene 的 parent 关系变了）。
            _eventHandler.SetHandle((System.IntPtr)_stage);

            // §4.5 stress fixture：勾选 → 程序生成 ~500 节点 html/css（mesh + text 双路径）。
            if (_stress500) BuildStress500Fixture();

            bool loaded;
            if (_usePackage)
            {
                loaded = LoadPackage();
            }
            else
            {
                loaded = LoadHtml();
            }
            if (!loaded)
            {
                Debug.LogError("[LoomStage] load 失败");
                FreeStage();
                return;
            }

            var shader = Shader.Find("LoomGUI/Unlit");
            if (shader == null)
            {
                Debug.LogError("[LoomStage] 找不到 Shader \"LoomGUI/Unlit\"——URP shader 是否在包内？");
                FreeStage();
                return;
            }
            _mm = new MaterialManager(shader);
            _pool = new MirrorPool();

            // v1b.3：collect atlas（atlas_count/info）→ load atlas.png → _texMap[atlas_tex_id]。
            // 同 atlas 所有 sprite 共享 1 Texture2D（batchable）。inline 分支（_usePackage=false）
            // 无 atlas 需加载——atlas_count 返 0，LoadAtlas 早退，_texMap 空（下游全 fallback）。
            if (_usePackage) LoadAtlas();

            EnsureFont();
            // Font.textureRebuilt 是静态事件（§4.3 必修坑）：atlas 异步 rebuild 时 glyph UV 变。
            // 注册 TextRasterizer.OnRebuilt（自增 FontVersion）→ MirrorPool.Sync 下帧检测到版本
            // 变 → 强制 text 节点重 RequestCharactersInTexture + 重取 UV（fgui DynamicFont.cs:356-375）。
            // 全局静态事件：必须 OnDestroy 解绑，否则泄漏跨场景/实例。
            Font.textureRebuilt += TextRasterizer.OnRebuilt;

            gameObject.layer = LoomUILayer;
            EnsureCamera();
            ConfigureTransforms();

            // v1c.1：_inputCollector 未在 Inspector 指定时，兜底取同 GO 上的组件
            // （常见用法：LoomInputCollector 挂在 LoomStage 同 GameObject）。
            if (_inputCollector == null) _inputCollector = GetComponent<LoomInputCollector>();
        }

        /// <summary>
        /// 取 Unity 动态 Font。Inspector 指定优先；未配则 EditMode 用 AssetDatabase 兜底加载
        /// Assets/LoomGUI/Fonts/DejaVuSans.ttf（PlayMode/build 必须由用户在 Inspector 指定——
        /// AssetDatabase 仅 editor）。Font 须与 Rust measure 同一份 ttf（§4.3 跨平台一致性根）。
        /// </summary>
        void EnsureFont()
        {
            if (_font != null) return;
#if UNITY_EDITOR
            _font = UnityEditor.AssetDatabase.LoadAssetAtPath<Font>("Assets/LoomGUI/Fonts/DejaVuSans.ttf");
            if (_font == null)
                Debug.LogError("[LoomStage] 未在 Inspector 指定 _font，且 Assets/LoomGUI/Fonts/DejaVuSans.ttf 不可达");
#else
            Debug.LogError("[LoomStage] PlayMode/build 必须在 Inspector 指定 _font（DejaVuSans）");
#endif
        }

        /// <summary>
        /// load_html：UTF8 字节 + fixed 钉住。返回 native 码（0=ok）。
        /// v1c.2-T5：load 成功后 SetHandle 清 handler._parentCache（scene 重建，parent 关系变）。
        /// </summary>
        bool LoadHtml()
        {
            if (_stage == null) return false;
            byte[] hb = string.IsNullOrEmpty(_html) ? Array.Empty<byte>() : Encoding.UTF8.GetBytes(_html);
            byte[] cb = string.IsNullOrEmpty(_css) ? Array.Empty<byte>() : Encoding.UTF8.GetBytes(_css);
            fixed (byte* hp = hb, cp = cb)
            {
                int r = Native.loomgui_stage_load_html(
                    _stage, hp, (nuint)hb.Length, cp, (nuint)cb.Length);
                if (r != 0) return false;
            }
            _eventHandler.SetHandle((System.IntPtr)_stage);
            return true;
        }

        /// <summary>
        /// §13 v1b.1：从 StreamingAssets/_pkgFile 读 .pkg.bin → loomgui_stage_load_package。
        /// 包是 Rust-internal，C# 只读文件透传 bytes（不解析）。editor/desktop 用 File.ReadAllBytes。
        /// v1c.2-T5：load 成功后 SetHandle 清 handler._parentCache（scene 重建，parent 关系变）。
        /// </summary>
        bool LoadPackage()
        {
            if (_stage == null) return false;
            string pkgPath = System.IO.Path.Combine(Application.streamingAssetsPath, _pkgFile);
            if (!System.IO.File.Exists(pkgPath))
            {
                Debug.LogError($"[LoomStage] 包文件不存在：{pkgPath}");
                return false;
            }
            byte[] pkg = System.IO.File.ReadAllBytes(pkgPath);
            fixed (byte* pp = pkg)
            {
                int r = Native.loomgui_stage_load_package(_stage, pp, (nuint)pkg.Length);
                if (r != 0) return false;
            }
            _eventHandler.SetHandle((System.IntPtr)_stage);
            return true;
        }

        /// <summary>
        /// v1b.3：collect atlas（atlas_count/info）→ 读 atlas.png → _texMap[atlas_tex_id]。
        /// 同 atlas 所有 sprite 共享 1 Texture2D（MaterialManager key=(program,tex,ctx) →
        /// 同 atlas 的多个节点复用同一 Material 实例 → batchable）。缺图/坏图 → LogError 跳过
        /// （下游 tex_id 缺 → texMap.TryGetValue miss → fallback 白占位，不阻塞渲染）。
        ///
        /// FFI 契约（T5 atlas_count/info）：
        ///   atlas_count(StageHandle*) → nuint（甲-B scene 恒 1；无图 scene = 0）。
        ///   atlas_info(StageHandle*, i, uint* tid, uint* w, uint* h, nuint* src_len) → byte*
        ///     返 atlas filename UTF-8 串（**无尾 NUL**）+ *out_src_len = 字节长；
        ///     *out_tex_id = core 分配（= i+1）；*out_w/*out_h = atlas 像素尺寸。
        ///
        /// T5 string contract（坑16）：返串**无尾 NUL** + out_len = 字节长。C# 必走
        /// `Encoding.UTF8.GetString(p, (int)srcLen)`（len-based 读），**禁止** PtrToStringAnsi
        /// （NUL-scan 会越过 stage 缓存末尾读未映射内存）。
        /// </summary>
        void LoadAtlas()
        {
            _texMap.Clear();
            if (_stage == null) return;

            nuint count;
            unsafe { count = Native.loomgui_stage_atlas_count(_stage); }

            for (nuint i = 0; i < count; i++)
            {
                byte* p = null;
                nuint srcLen = 0;
                uint tid = 0;
                uint aw = 0;
                uint ah = 0;
                unsafe { p = Native.loomgui_stage_atlas_info(_stage, i, &tid, &aw, &ah, &srcLen); }
                if (p == null || srcLen == 0) continue;

                // len-based 读（T5 contract；禁止 NUL-scan / PtrToStringAnsi）。
                string src = Encoding.UTF8.GetString(p, (int)srcLen);
                string path = System.IO.Path.Combine(Application.streamingAssetsPath, src);

                byte[] bytes;
                try { bytes = System.IO.File.ReadAllBytes(path); }
                catch (System.Exception e)
                {
                    Debug.LogError($"[LoomStage] atlas not found: {src} ({e.Message})");
                    continue;
                }

                // 初始尺寸传 atlas 元数据 w/h（LoadImage 会按 PNG IHDR 重设，但构造时给合理值）。
                var tex = new Texture2D((int)aw, (int)ah);
                if (!tex.LoadImage(bytes))
                {
                    Debug.LogError($"[LoomStage] bad atlas png: {src}");
                    // 全限定 UnityEngine.Object：using System 引入 System.Object，裸 Object 歧义（坑18）。
                    if (Application.isPlaying) UnityEngine.Object.Destroy(tex);
                    else UnityEngine.Object.DestroyImmediate(tex);
                    continue;
                }

                _texMap[tid] = tex;   // tid = i+1（core 分配）；同 atlas 所有 sprite 共享此 Texture2D
            }
        }

        /// <summary>
        /// §4.5 stress fixture：程序生成 ~500 渲染节点的 html/css。
        /// 结构：一个 flex column 容器，内含 N 行；每行 = 一个 colored div（mesh 路径）+
        /// 一个 text label（text 路径）。每行约 2 渲染节点（div + text），250 行 ≈ 500 节点。
        /// 颜色用行号取模分配（视觉可辨、避免全同色无法肉眼判卡顿）。
        /// v0 解析器不读 inline style=（layout/mod.rs 注释），故全用 class + 独立 CSS。
        /// </summary>
        void BuildStress500Fixture()
        {
            const int Rows = 250;   // 每行 1 div + 1 text 子 → ~500 渲染节点。
            var html = new StringBuilder(1 << 14);
            html.Append("<div class=\"c\">");
            for (int i = 0; i < Rows; i++)
            {
                // 裸文本（非 <p>——p 不在 v1 围栏元素 div/span/img/button 内，parse 白名单拒
                // → load 失败 0 节点）。div 裸文本 → Text 子节点（scene/node.rs::build_text_child）。
                html.Append("<div class=\"r").Append(i % 4).Append("\">row ").Append(i).Append("</div>");
            }
            html.Append("</div>");

            // color/font-size 放 .c（继承到所有 text 子）；.rX 只管配色/尺寸/margin。
            // 用 px 绝对值（v0 layout 支持）；行高 ~32px 容纳 250 行（超出 design 高度会溢出，
            // 但本测关心的是渲染节点数与帧时间，不关心可视区域；Rust 仍 layout 全部节点）。
            var css = new StringBuilder(1 << 12);
            css.Append(".c{display:flex;flex-direction:column;width:1000px;color:#ffffff;font-size:20px;}");
            css.Append(".r0{width:960px;height:28px;background-color:#c62828;margin:2px;}");
            css.Append(".r1{width:960px;height:28px;background-color:#1565c0;margin:2px;}");
            css.Append(".r2{width:960px;height:28px;background-color:#2e7d32;margin:2px;}");
            css.Append(".r3{width:960px;height:28px;background-color:#6a1b9a;margin:2px;}");

            _html = html.ToString();
            _css = css.ToString();
        }

        /// <summary>
        /// §4.5 on-screen FPS 读数（_stress500 或 _showFps 任一为真时显示）。
        /// 1/Time.smoothDeltaTime 平滑帧率 + MirrorPool 当前节点数。最小实现（不做 profiler）。
        /// 用户在 PlayMode 肉眼判卡顿（v1a §9.3 便宜帧 ≥45fps 静态无卡顿）。
        /// </summary>
        void OnGUI()
        {
            if (!_stress500 && !_showFps) return;
            float fps = Time.smoothDeltaTime > 0f ? 1f / Time.smoothDeltaTime : 0f;
            int nodes = _pool?.Count ?? 0;
            string label = $"FPS {fps:F1}  nodes {nodes}";
            GUI.Label(new Rect(8f, 8f, 240f, 24f), label);
        }

        /// <summary>
        /// 建/取 UI 相机。独立 GO（非根的子节点）——避免被根的 (sf,-sf,sf) scale 影响。
        /// 用户在 Inspector 指定优先；否则现场建一个。
        /// </summary>
        void EnsureCamera()
        {
            if (_uiCamera == null)
            {
                var cgo = new GameObject("LoomUICamera");
                _uiCamera = cgo.AddComponent<Camera>();
                // URP：附加 UniversalAdditionalCameraData（若有该类型）。
                // 用反射避免硬引用 URP 程序集；缺失则跳过（用户可手挂）。
                try
                {
                    var t = Type.GetType("UnityEngine.Rendering.Universal.UniversalAdditionalCameraData, Unity.RenderPipelines.Universal.Runtime");
                    if (t != null && _uiCamera.GetComponent(t) == null)
                        _uiCamera.gameObject.AddComponent(t);
                }
                catch { /* URP 缺失：忽略 */ }
            }
            _uiCamera.gameObject.layer = LoomUILayer;
        }

        /// <summary>
        /// 根 Stage transform：scale=(sf,-sf,sf)、position=(-sw/2, sh/2, 0)。
        /// UI 相机：正交、orthoSize=sh/2、cullingMask=1&lt;&lt;LoomUI、pos=(0,0,-10) 看向 z=0。
        /// 校验：design 点 (dx,dy) → 世界 (-sw/2 + dx·sf, sh/2 − dy·sf)。
        /// </summary>
        void ConfigureTransforms()
        {
            float sw = Screen.width, sh = Screen.height;
            // 注：这是 shrink-to-fit（取较小缩放比，保证完整可见 + 留白 letterbox），
            // ≈ CanvasScaler MatchWidthOrHeight 在 match≈0.5 但带 letterboxing，
            // 并非字面意义的 MatchWidthOrHeight 插值缩放。v1d responsive 再重审（可能改为 cover/contain 选项）。
            float sf = Mathf.Min(sw / _designSize.x, sh / _designSize.y);

            transform.localScale = new Vector3(sf, -sf, sf);
            transform.localPosition = new Vector3(-sw / 2f, sh / 2f, 0f);

            if (_uiCamera != null)
            {
                _uiCamera.orthographic = true;
                _uiCamera.orthographicSize = sh / 2f;
                _uiCamera.cullingMask = 1 << LoomUILayer;
                _uiCamera.clearFlags = CameraClearFlags.Depth;
                _uiCamera.nearClipPlane = 0.1f;   // Unity 要求 near>0；相机 z=-10 看向 z=0 内容
                _uiCamera.farClipPlane = 100f;
                // 相机独立于根（不 SetParent）：放世界 (0,0,-10) 看向 +z，content 在 z=0。
                _uiCamera.transform.SetParent(null, false);
                _uiCamera.transform.localPosition = new Vector3(0f, 0f, -10f);
                _uiCamera.transform.localRotation = Quaternion.identity;
            }
        }

        void LateUpdate()
        {
            if (_stage == null) return;

            // 屏幕 resize 检测（editor 改 Game 视图尺寸 / player 改窗口）。
            if (Screen.width != _lastScreenW || Screen.height != _lastScreenH)
            {
                _lastScreenW = Screen.width;
                _lastScreenH = Screen.height;
                ConfigureTransforms();
            }

            // v1c.1：输入采集 → set_input（tick 前——input 管线消费本帧输入产事件）。
            if (_inputCollector != null)
                _inputCollector.Collect((System.IntPtr)_stage, _designSize);

            // tick → build_blob 写入 Rust 拥有缓存（dt v0 忽略）。
            Native.loomgui_stage_tick(_stage, Time.deltaTime);

            // borrow_frame：返回 byte*（缓存首），写 nuint 长度。
            // 局部变量已在栈上固定，直接 & 取址传入（fixed 反而报 CS0213 "already fixed"）。
            nuint lenRaw = 0;
            byte* ptr = Native.loomgui_stage_borrow_frame(_stage, &lenRaw);
            int len = (int)lenRaw;
            if (ptr != null && len > 0)
            {
                // 原子拷贝到托管 buffer（§14.3）。v1a 先 new；ArrayPool 留 v1e。
                if (_frameBuf == null || _frameBuf.Length < len)
                    _frameBuf = new byte[len];
                Marshal.Copy((IntPtr)ptr, _frameBuf, 0, len);

                var blob = new FrameBlob(_frameBuf);
                _pool.Sync(blob, transform, _mm, _texMap, Texture2D.whiteTexture, _font);
            }

            // v1c.1：事件派发（tick 后——borrow_events 读本帧 last_events，下 tick 失效）。
            // 即使 borrow_frame 为空（无渲染节点），事件仍须派发（hover/点击不依赖渲染）。
            if (_eventHandler != null)
            {
                nuint evLen = 0;
                byte* evPtr = Native.loomgui_stage_borrow_events(_stage, &evLen);
                _eventHandler.DispatchPending((System.IntPtr)evPtr, (int)evLen);
            }
        }

        void OnDestroy()
        {
            // 全局静态事件：Awake 注册过才解绑（Awake 失败早退则跳过）。
            Font.textureRebuilt -= TextRasterizer.OnRebuilt;
            _pool?.Clear();
            _mm?.Clear();
            // v1b.2：Dispose 真纹理。ExecuteAlways 下 OnDestroy 在 Edit/Play 都会跑——
            // 必须按 Application.isPlaying 选 Destroy / DestroyImmediate（同 MirrorPool.TearDown 模式）。
            if (_texMap != null)
            {
                foreach (var t in _texMap.Values)
                {
                    if (Application.isPlaying) UnityEngine.Object.Destroy(t);
                    else UnityEngine.Object.DestroyImmediate(t);
                }
                _texMap.Clear();
            }
            FreeStage();
        }

        void FreeStage()
        {
            if (_stage != null)
            {
                Native.loomgui_stage_free(_stage); // null-safe（native 侧检查）
                _stage = null;
            }
        }

        // Domain reload 保护（§4.3e / §4.6 / G13，照 fgui Stage.cs:86）。SubsystemRegistration 在
        // Domain reload 时跑（关闭 Domain Reload 仍跑——这正是本 hook 存在的根因：关 reload 时 C#
        // 静态活过 Play，但 native 状态可能悬空）。Phase 2：
        //   1. Native.loomgui_shutdown() — native 全局态当前为空（Stage per-handle，stage_free drop），
        //      但 hook 必须接——v1b 引入 global texture/font registry 时此处自动清，无需再改接线。
        //      （注意：Font 的 Box::leak 是真泄漏，每次 Stage 创建 leak 一份字体字节——不可由 shutdown
        //      回收，需字体缓存化才能根治。×20 域重载测观察内存增长决定是否 Phase 2 内做。）
        //   2. TextRasterizer.ResetStatic() — 清 C# 静态 s_fontVersion（atlas rebuild 计数器）。
        //   （MaterialManager/MirrorPool 都是 per-instance，随 MonoBehaviour OnDestroy 销毁，无 static。）
        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]
        static void ResetStatics()
        {
            Native.loomgui_shutdown();
            TextRasterizer.ResetStatic();
        }
    }
}
