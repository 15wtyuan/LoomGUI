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

            gameObject.layer = LoomUILayer;
            EnsureCamera();
            ConfigureTransforms();
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

            // borrow_frame：返回 byte*（缓存首），写 nuint 长度。钉一个 nuint 取回。
            nuint lenRaw = 0;
            byte* ptr;
            fixed (nuint* lp = &lenRaw)
            {
                ptr = Native.loomgui_stage_borrow_frame(_stage, lp);
            }
            if (ptr == null || lenRaw == 0) return;

            int len = (int)lenRaw;
            // 原子拷贝到托管 buffer（§14.3）。v1a 先 new；ArrayPool 留 v1e。
            if (_frameBuf == null || _frameBuf.Length < len)
                _frameBuf = new byte[len];
            Marshal.Copy((IntPtr)ptr, _frameBuf, 0, len);

            var blob = new FrameBlob(_frameBuf);
            _pool.Sync(blob, transform, _mm, Texture2D.whiteTexture);
        }

        void OnDestroy()
        {
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

        // Domain reload 保护（§4.3e）。Phase 1 最小：占位。Phase 2 调 loomgui_shutdown + 清全局。
        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]
        static void ResetStatics() { /* Phase 2 */ }
    }
}
