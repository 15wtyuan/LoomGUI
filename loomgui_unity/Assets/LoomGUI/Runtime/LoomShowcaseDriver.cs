using UnityEngine;

namespace LoomGUI
{
    // v1 showcase driver: nav 跳转 + 启动错峰入场 + 交互灯阵（T7）+ tween 演示（T8）。
    public unsafe class LoomShowcaseDriver : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;
        [SerializeField] float[] _sectionY = { 0f, 700f, 1500f, 2300f, 3000f, 3800f, 4500f, 5200f }; // 设计期累积高度（sec-1..sec-8 顶部 y）

        uint _scrollNode = uint.MaxValue;
        uint[] _navNodes = new uint[8];

        // === 灯阵（T7 §4）===
        int _clickCount, _hoverCount, _dragCount, _longCount, _keyCount, _routeCount;

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[Showcase] 无 LoomStage"); return; }
        }

        void Start()
        {
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
        }

        // design-taste §4: 启动错峰入场。各 sec 卡 tween opacity 0→1 + delay 递增。
        // 注：HTML 各 sec 初始 opacity:0（见 style.css .sec），靠 tween 拉亮；同时验证 §7.1 opacity + §7.3 delay。
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

        // === §4 灯阵订阅（T7）===
        // 订阅各交互元素事件 + 禁用 hit-disabled。pointer-events/StopProp 在 4.7 外/内 Click 上演示。
        void SubscribeLampEvents()
        {
            SubscribeLamp("hit-click", EventType.Click, OnClickHit);
            SubscribeLamp("hit-hover", EventType.RollOver, OnHoverHit);
            SubscribeLamp("hit-hover", EventType.RollOut, OnHoverLeave);
            SubscribeLamp("hit-drag", EventType.DragMove, OnDragHit);
            SubscribeLamp("hit-longpress", EventType.LongPress, OnLongHit);
            SubscribeLamp("hit-key", EventType.KeyDown, OnKeyHit);
            // 4.3 disabled：Start 设 disabled，click 不触发。
            uint dn = _stage.FindNodeById("hit-disabled");
            if (dn != uint.MaxValue) _stage.SetNodeDisabled(dn, true);
            // 4.7 路由：outer/inner 均订阅 Click；inner 调 StopPropagation 止冒泡（outer 不触发）。
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

        // 点亮 lamp-{name} 容器（ponytail: 无 get_children API，改用整容器 opacity 脉冲指示触发）。
        // 每触发 tween 1→0.3 闪一下，作为触发反馈；放弃逐盏计数可视化（v1 简化）。
        void LightLamp(string name, int count)
        {
            uint container = _stage.FindNodeById("lamp-" + name);
            if (container == uint.MaxValue) return;
            _stage.Tween(container, TweenProp.Opacity,
                new float[] { 1f, 0, 0, 0 }, new float[] { 0.3f, 0, 0, 0 },
                0.2f, Ease.QuadOut, 0f, 0);
        }

        // 4.1 click + dblclick：双击额外多亮一盏（用 acc 色标记）。
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

        // 4.7 路由演示：inner StopPropagation → outer 不收。独立 lamp-route 反馈。
        void OnRouteOuter(EventContext ctx) { LightLamp("route", ++_routeCount); }   // 外层命中（inner 未止冒泡时）
        void OnRouteInner(EventContext ctx)
        {
            ctx.StopPropagation();   // 止冒泡，outer 不触发
            LightLamp("route", ++_routeCount);
        }
        void OnRoutePe(EventContext ctx) { LightLamp("route", ++_routeCount); }   // pointer-events:none 穿透后命中下层
    }
}
