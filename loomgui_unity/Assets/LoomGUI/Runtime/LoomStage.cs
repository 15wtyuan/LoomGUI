using System;
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

        // 上一帧 Screen 尺寸，检测 resize 重配根/相机。
        int _lastScreenW = -1, _lastScreenH = -1;

        const int LoomUILayer = 6;

        void Awake()
        {
            // Stage::new 需字体路径（即使纯色块场景也要加载用于 measure）。
            // Application.streamingAssetsPath：editor 与 player 都可用（editor 返回 Assets/StreamingAssets）。
            string fontPath = System.IO.Path.Combine(Application.streamingAssetsPath, "DejaVuSans.ttf");
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

            // §4.5 stress fixture：勾选 → 程序生成 ~500 节点 html/css（mesh + text 双路径）。
            if (_stress500) BuildStress500Fixture();

            if (!LoadHtml())
            {
                Debug.LogError("[LoomStage] load_html 失败");
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

            EnsureFont();
            // Font.textureRebuilt 是静态事件（§4.3 必修坑）：atlas 异步 rebuild 时 glyph UV 变。
            // 注册 TextRasterizer.OnRebuilt（自增 FontVersion）→ MirrorPool.Sync 下帧检测到版本
            // 变 → 强制 text 节点重 RequestCharactersInTexture + 重取 UV（fgui DynamicFont.cs:356-375）。
            // 全局静态事件：必须 OnDestroy 解绑，否则泄漏跨场景/实例。
            Font.textureRebuilt += TextRasterizer.OnRebuilt;

            gameObject.layer = LoomUILayer;
            EnsureCamera();
            ConfigureTransforms();
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
                return r == 0;
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
            const int Rows = 250;   // 每行 1 div + 1 text → ~500 渲染节点。
            var html = new StringBuilder(1 << 14);
            html.Append("<div class=\"c\">");
            for (int i = 0; i < Rows; i++)
            {
                // 每行一个带 class 的 div + 一个 <p> 文本节点。
                html.Append("<div class=\"r").Append(i % 4).Append("\"><p>row ").Append(i).Append("</p></div>");
            }
            html.Append("</div>");

            // CSS：容器 flex column 定宽；4 种行 class 循环配色 + p 文本样式。
            // 用 px 绝对值（v0 layout 支持）；行高 ~30px 容纳 250 行（超出 design 高度会溢出，
            // 但本测关心的是渲染节点数与帧时间，不关心可视区域；Rust 仍 layout 全部节点）。
            var css = new StringBuilder(1 << 12);
            css.Append(".c{display:flex;flex-direction:column;width:1000px;}");
            css.Append(".r0{width:960px;height:28px;background-color:#c62828;margin:2px;}");
            css.Append(".r1{width:960px;height:28px;background-color:#1565c0;margin:2px;}");
            css.Append(".r2{width:960px;height:28px;background-color:#2e7d32;margin:2px;}");
            css.Append(".r3{width:960px;height:28px;background-color:#6a1b9a;margin:2px;}");
            css.Append("p{font-size:20px;color:#ffffff;}");

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

            // tick → build_blob 写入 Rust 拥有缓存（dt v0 忽略）。
            Native.loomgui_stage_tick(_stage, Time.deltaTime);

            // borrow_frame：返回 byte*（缓存首），写 nuint 长度。
            // 局部变量已在栈上固定，直接 & 取址传入（fixed 反而报 CS0213 "already fixed"）。
            nuint lenRaw = 0;
            byte* ptr = Native.loomgui_stage_borrow_frame(_stage, &lenRaw);
            if (ptr == null || lenRaw == 0) return;

            int len = (int)lenRaw;
            // 原子拷贝到托管 buffer（§14.3）。v1a 先 new；ArrayPool 留 v1e。
            if (_frameBuf == null || _frameBuf.Length < len)
                _frameBuf = new byte[len];
            Marshal.Copy((IntPtr)ptr, _frameBuf, 0, len);

            var blob = new FrameBlob(_frameBuf);
            _pool.Sync(blob, transform, _mm, Texture2D.whiteTexture, _font);
        }

        void OnDestroy()
        {
            // 全局静态事件：Awake 注册过才解绑（Awake 失败早退则跳过）。
            Font.textureRebuilt -= TextRasterizer.OnRebuilt;
            _pool?.Clear();
            _mm?.Clear();
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
