using UnityEngine;

namespace LoomGUI
{
    // showcase driver：nav 跳转 + 启动错峰入场 + 交互灯阵 + tween 演示。
    public unsafe class LoomShowcaseDriver : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;
        // 各 sec 顶部 y（设计期累积高度；sec-7/8 含新增卡，sec-8 顶≈4500+sec-7 高）。
        [SerializeField] float[] _sectionY = { 0f, 700f, 1500f, 2300f, 3000f, 3800f, 4500f, 5600f };
        // 外部 GO 绑 model-slot（Inspector 拖 Cube 等）。
        [SerializeField] GameObject _nativeModel;
        // Cube 1m³ 在 UI design 空间天然小（跟 fgui GoWrapper 一致），设 scale 放大填 slot。
        // NativeHost Sync 设 per-node wrapper、不动用户 GO scale。
        [SerializeField] Vector3 _nativeScale = new Vector3(120, 120, 120);

        uint _scrollNode = uint.MaxValue;
        uint[] _navNodes = new uint[8];

        // === 灯阵 ===
        int _clickCount, _hoverCount, _dragCount, _longCount, _keyCount, _routeCount;

        // === tween 演示 ===
        // Ease 0..9 与 Rust tween::Ease 对齐（OnEasePlay 取子集对比）。六 prop 在 OnTweenPlay 逐个硬编码 PlayProp。
        static readonly Ease[] _allEase = { Ease.Linear, Ease.QuadIn, Ease.QuadOut, Ease.QuadInOut, Ease.CubicIn, Ease.CubicOut, Ease.CubicInOut, Ease.BackIn, Ease.BackOut, Ease.BackInOut };
        const uint TagComplete = 7;   // complete 回调用 tag

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[Showcase] 无 LoomStage"); return; }
        }

        // #1a1d2e = .root 背景色（showcase 深蓝底）。主相机配同色，letterbox 与 root 无缝。
        static readonly Color RootBg = new Color(26f / 255f, 29f / 255f, 46f / 255f, 1f);

        void Start()
        {
            ConfigureCameraBackground();
            _scrollNode = _stage.FindNodeById("main-scroll");
            for (int i = 0; i < 8; i++)
            {
                _navNodes[i] = _stage.FindNodeById("nav-" + (i + 1));
                if (_navNodes[i] != uint.MaxValue)
                    _stage.EventHandler.AddListener(_navNodes[i], EventType.Click, OnNavClick);
            }
            Debug.Log($"[Showcase] scroll={_scrollNode} nav0={_navNodes[0]}（点 nav 跳区）");
            StaggeredEntrance();
            SubscribeLampEvents();
            SubscribeTweenDemos();
            // 绑外部 GO 到 model-slot（每帧 Sync 自动同步 wrapper TRS；GO 自身 scale 保留）。
            if (_nativeModel != null)
            {
                _stage.BindNativeHost("model-slot", _nativeModel);
                _nativeModel.transform.localScale = _nativeScale;  // demo 放大填 slot
            }
        }

        // 启动错峰入场：各 sec 卡 tween opacity 0→1 + delay 递增。
        // HTML 各 sec 初始 opacity:0（见 style.css .sec），靠 tween 拉亮；同时验证 opacity prop + delay。
        void StaggeredEntrance()
        {
            for (int i = 0; i < 8; i++)
            {
                uint node = _stage.FindNodeById("sec-" + (i + 1));
                if (node == uint.MaxValue) continue;
                _stage.Tween(node, TweenProp.Opacity,
                    new float[] { 0f, 0, 0, 0 }, new float[] { 1f, 0, 0, 0 },
                    0.4f, Ease.CubicOut, i * 0.04f, 0);
            }
        }

        // 主相机默认 Skybox（蓝灰渐变）；root shrink-to-fit + safeArea letterbox 后透出 → 整体灰蒙蒙。
        // LoomUICamera clearFlags=Depth 不清色、叠在主相机上。改主相机纯色 = root bg，letterbox 统一深色。
        void ConfigureCameraBackground()
        {
            var cam = Camera.main;
            if (cam != null)
            {
                cam.clearFlags = CameraClearFlags.SolidColor;
                cam.backgroundColor = RootBg;
            }
        }

        void OnNavClick(EventContext ctx)
        {
            if (_scrollNode == uint.MaxValue) return;
            for (int i = 0; i < 8; i++)
            {
                if (ctx.target == _navNodes[i])
                {
                    _stage.SetScrollPos(_scrollNode, 0f, _sectionY[i], true);
                    return;
                }
            }
        }

        // === 灯阵订阅 ===
        // 订阅各交互元素事件 + 禁用 hit-disabled。pointer-events/StopProp 在外/内 Click 上演示。
        void SubscribeLampEvents()
        {
            SubscribeLamp("hit-click", EventType.Click, OnClickHit);
            SubscribeLamp("hit-hover", EventType.RollOver, OnHoverHit);
            SubscribeLamp("hit-hover", EventType.RollOut, OnHoverLeave);
            SubscribeLamp("hit-drag", EventType.DragMove, OnDragHit);
            SubscribeLamp("hit-longpress", EventType.LongPress, OnLongHit);
            SubscribeLamp("hit-key", EventType.KeyDown, OnKeyHit);
            // disabled：Start 设 disabled，click 不触发。
            uint dn = _stage.FindNodeById("hit-disabled");
            if (dn != uint.MaxValue) _stage.SetNodeDisabled(dn, true);
            // btn-demo disabled：CSS .disabled class 只给样式（灰底+opacity），运行时 disabled
            // 靠 SetNodeDisabled（LoomGUI disabled 是 API 驱动，非 CSS class）。漏调则按下 :active 仍匹配 → 变蓝。
            uint dbd = _stage.FindNodeById("btn-demo-disabled");
            if (dbd != uint.MaxValue) _stage.SetNodeDisabled(dbd, true);
            // 路由：outer/inner 均订阅 Click；inner 调 StopPropagation 止冒泡（outer 不触发）。
            SubscribeLamp("route-outer", EventType.Click, OnRouteOuter);
            SubscribeLamp("route-inner", EventType.Click, OnRouteInner);
            SubscribeLamp("route-pe", EventType.Click, OnRoutePe);
            Debug.Log("[Showcase] §4 灯阵订阅完成（click/hover/drag/longpress/key + route + disabled）");
        }

        void SubscribeLamp(string id, EventType t, EventCallback cb)
        {
            uint n = _stage.FindNodeById(id);
            if (n != uint.MaxValue) _stage.EventHandler.AddListener(n, t, cb);
        }

        // 点亮 lamp-{name} 容器：无 get_children API，改用整容器 opacity 脉冲指示触发。
        // 每触发 tween 1→0.3 闪一下作为反馈。
        void LightLamp(string name, int count)
        {
            uint container = _stage.FindNodeById("lamp-" + name);
            if (container == uint.MaxValue) return;
            _stage.Tween(container, TweenProp.Opacity,
                new float[] { 1f, 0, 0, 0 }, new float[] { 0.3f, 0, 0, 0 },
                0.2f, Ease.QuadOut, 0f, 0);
        }

        // click + dblclick：双击额外多亮一盏（用 acc 色标记）。
        void OnClickHit(EventContext ctx)
        {
            LightLamp("click", ++_clickCount);
            if (ctx.isDoubleClick) LightLamp("click", ++_clickCount);   // 双击闪两下区分
        }
        void OnHoverHit(EventContext ctx) { LightLamp("hover", ++_hoverCount); }
        void OnHoverLeave(EventContext ctx) { LightLamp("hover", ++_hoverCount); }
        void OnDragHit(EventContext ctx) { LightLamp("drag", ++_dragCount); }
        void OnLongHit(EventContext ctx) { LightLamp("longpress", ++_longCount); }
        void OnKeyHit(EventContext ctx) { LightLamp("key", ++_keyCount); }

        // 路由演示：inner StopPropagation → outer 不收。独立 lamp-route 反馈。
        void OnRouteOuter(EventContext ctx) { LightLamp("route", ++_routeCount); }   // 外层命中（inner 未止冒泡时）
        void OnRouteInner(EventContext ctx)
        {
            ctx.StopPropagation();   // 止冒泡，outer 不触发
            LightLamp("route", ++_routeCount);
        }
        void OnRoutePe(EventContext ctx) { LightLamp("route", ++_routeCount); }   // pointer-events:none 穿透后命中下层

        // === tween 演示订阅 ===
        void SubscribeTweenDemos()
        {
            SubscribeLamp("tween-play", EventType.Click, OnTweenPlay);
            SubscribeLamp("ease-play", EventType.Click, OnEasePlay);
            SubscribeLamp("delay-play", EventType.Click, OnDelayPlay);
            SubscribeLamp("complete-play", EventType.Click, OnCompletePlay);
            SubscribeLamp("kill-btn", EventType.Click, OnKill);
            SubscribeLamp("clear-btn", EventType.Click, OnClear);
            // 监听 t-opacity 的 TweenComplete（core 完成时直派，ctx.clickCount=prop、ctx.touchId=tag）。
            SubscribeLamp("t-opacity", EventType.TweenComplete, OnTweenCompleteTag);
            // kill-target：启动即开始持续旋转（单次长 tween——loop 需 TweenComplete 重启，简化省略）。
            PlayProp("kill-target", TweenProp.Rotation, new float[] { 0f, 0, 0, 0 }, new float[] { 360f, 0, 0, 0 }, 4f, Ease.Linear, 0f, 0);
            Debug.Log("[Showcase] §7 tween 演示订阅完成（play/ease/delay/complete/kill/clear + kill-target 旋转）");
        }

        void PlayProp(string id, TweenProp prop, float[] s, float[] e, float dur, Ease ease, float delay, uint tag)
        {
            uint n = _stage.FindNodeById(id);
            if (n != uint.MaxValue) _stage.Tween(n, prop, s, e, dur, ease, delay, tag);
        }

        // 六属性同放：opacity / translate / scale / rotation / bg-color / text-color。
        void OnTweenPlay(EventContext ctx)
        {
            PlayProp("t-opacity", TweenProp.Opacity, new float[] { 0f, 0, 0, 0 }, new float[] { 1f, 0, 0, 0 }, 0.8f, Ease.Linear, 0f, 0);
            PlayProp("t-translate", TweenProp.Translate, new float[] { -40f, 0, 0, 0 }, new float[] { 40f, 0, 0, 0 }, 0.8f, Ease.CubicInOut, 0f, 0);
            PlayProp("t-scale", TweenProp.Scale, new float[] { 0.5f, 0.5f, 0, 0 }, new float[] { 1.4f, 1.4f, 0, 0 }, 0.8f, Ease.BackOut, 0f, 0);
            PlayProp("t-rotate", TweenProp.Rotation, new float[] { 0f, 0, 0, 0 }, new float[] { 360f, 0, 0, 0 }, 0.8f, Ease.QuadInOut, 0f, 0);
            // 颜色 tween：Rust anim 通道是归一化 [0,1] RGBA（style/mapping.rs /255.0），故 float[] 也须归一化。
            PlayProp("t-bgcolor", TweenProp.BgColor, Rgba(0x5f, 0xb2, 0xc4), Rgba(0x6f, 0xa6, 0x6c), 0.8f, Ease.Linear, 0f, 0);
            PlayProp("t-textcolor", TweenProp.TextColor, Rgba(0xe6, 0xe6, 0xe0), Rgba(0xc2, 0x60, 0x5a), 0.8f, Ease.Linear, 0f, 0);
        }

        // 三条 ease 对比（QuadIn / CubicOut / BackInOut），同 translate 200px。
        void OnEasePlay(EventContext ctx)
        {
            int[] pick = { 1, 5, 9 };   // _allEase 下标：QuadIn / CubicOut / BackInOut
            for (int i = 0; i < pick.Length; i++)
                PlayProp("ease-" + i, TweenProp.Translate, new float[] { 0f, 0, 0, 0 }, new float[] { 200f, 0, 0, 0 }, 1.0f, _allEase[pick[i]], 0f, 0);
        }

        // delay 错峰：三块依次起（= 启动 StaggeredEntrance 机制）。
        void OnDelayPlay(EventContext ctx)
        {
            PlayProp("d-0", TweenProp.Opacity, new float[] { 0f, 0, 0, 0 }, new float[] { 1f, 0, 0, 0 }, 0.5f, Ease.CubicOut, 0f, 0);
            PlayProp("d-1", TweenProp.Opacity, new float[] { 0f, 0, 0, 0 }, new float[] { 1f, 0, 0, 0 }, 0.5f, Ease.CubicOut, 0.2f, 0);
            PlayProp("d-2", TweenProp.Opacity, new float[] { 0f, 0, 0, 0 }, new float[] { 1f, 0, 0, 0 }, 0.5f, Ease.CubicOut, 0.4f, 0);
        }

        // complete：t-opacity 跑完后 core 派 TweenComplete（tag=TagComplete），C# 识别 tag 亮灯。
        void OnCompletePlay(EventContext ctx)
        {
            PlayProp("t-opacity", TweenProp.Opacity, new float[] { 1f, 0, 0, 0 }, new float[] { 0.2f, 0, 0, 0 }, 0.6f, Ease.QuadIn, 0f, TagComplete);
        }
        void OnTweenCompleteTag(EventContext ctx)
        {
            // ctx.clickCount = prop（Opacity=0）；ctx.touchId = tag（TagComplete=7）。
            if (ctx.touchId == TagComplete) LightLamp("complete", 1);
        }

        // kill 冻结当前角（停在末值）；clear 清所有 anim 回 CSS 初始。
        void OnKill(EventContext ctx) { _stage.KillTween(_stage.FindNodeById("kill-target"), TweenProp.Rotation); }
        void OnClear(EventContext ctx) { _stage.ClearAnim(_stage.FindNodeById("kill-target")); }

        // 0-255 RGB → 归一化 [0,1] RGBA float[4]（alpha=1）。Rust tween 直接写 anim 通道，须与 style 归一化一致。
        static float[] Rgba(int r, int g, int b) => new float[] { r / 255f, g / 255f, b / 255f, 1f };
    }
}
