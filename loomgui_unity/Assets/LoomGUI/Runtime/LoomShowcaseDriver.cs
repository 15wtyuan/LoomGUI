using System.Collections.Generic;
using UnityEngine;

namespace LoomGUI
{
    // showcase driver（v1.4-a T11 重写）：layer 骨架 + web 式多页导航 + 按页 listener 清 + tips 叠加。
    //
    // 模型（spec §7.3-7.5）：
    //   Start 建 root + ui_layer（主界面层）+ tips_layer（tips 层，在上）；
    //   LoadPackage("showcase", bytes) 进资源池；Instantiate("showcase","home") 挂 ui_layer。
    //   home nav 按钮 → OpenPage(目标页)：清当前页 listener + RemoveNode(当前页) + Instantiate(目标) + 挂 ui_layer + SubscribePage。
    //   各页 back-home → OpenPage("home")。tips_toast 演示 → Instantiate 挂 tips_layer + 定时 RemoveNode。
    //   dyn-load-mail → Instantiate("showcase","mail") 挂 ui_layer（叠加，非切包）；dyn-load-showcase → RemoveNode(mail)。
    //
    // 按页 listener 清（§7.5）：driver 维护当前页 listener 注册表，切页前批量 RemoveListener（不用 EventHandler.Clear 粗清）。
    public unsafe class LoomShowcaseDriver : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;
        // 外部 GO 绑 model-slot（page_controls §1.6 NativeHost 演示；Inspector 拖 Cube 等）。
        [SerializeField] GameObject _nativeModel;
        // Cube 1m³ 在 UI design 空间天然小，设 scale 放大填 slot（NativeHost Sync 不动用户 GO scale）。
        [SerializeField] Vector3 _nativeScale = new Vector3(120, 120, 120);

        // layer 骨架 NodeId
        uint _root = uint.MaxValue;
        uint _uiLayer = uint.MaxValue;
        uint _tipsLayer = uint.MaxValue;

        // 当前页根 NodeId（home 初始）；uint.MaxValue = 未建。
        uint _currentPage = uint.MaxValue;
        // mail 叠加层 NodeId（dyn-load-mail instantiate 出的 mail 组件根；uint.MaxValue = 未挂）。
        uint _mailOverlay = uint.MaxValue;

        // showcase 包名（LoadPackage 用）+ pkg.bin 文件名（StreamingAssets 下）。
        const string ShowcasePkg = "showcase";
        const string ShowcasePkgFile = "loom_showcase.pkg.bin";

        // === 按页 listener 注册表（§7.5）===
        // 当前页注册的 listener：nodeId → [(eventType, callback)]。切页前遍历逐个 RemoveListener。
        readonly Dictionary<uint, List<(EventType type, EventCallback cb)>> _pageListeners = new();

        // === 灯阵计数（page_interact）===
        int _clickCount, _hoverCount, _dragCount, _longCount, _keyCount, _routeCount;

        // === tween 演示（page_tween）===
        // Ease 0..9 与 Rust tween::Ease 对齐（OnEasePlay 取子集对比）。六 prop 在 OnTweenPlay 逐个硬编码 PlayProp。
        static readonly Ease[] _allEase = { Ease.Linear, Ease.QuadIn, Ease.QuadOut, Ease.QuadInOut, Ease.CubicIn, Ease.CubicOut, Ease.CubicInOut, Ease.BackIn, Ease.BackOut, Ease.BackInOut };
        const uint TagComplete = 7;   // complete 回调用 tag

        // === 动态树演示（page_dyntree §3.10）===
        // dyn-anchor 是 pkg 里的空容器；点击 dyn-add 运行时 create_node 建 panel+title+icon 挂到 anchor。
        // _dynPanels 记已建 panel NodeId 栈，dyn-del remove 最后一个。
        uint _dynAnchor = uint.MaxValue;
        readonly List<uint> _dynPanels = new();
        int _dynSeq;
        bool _dynStyleToggled;   // toggle 末个 panel 样式状态

        // === tips 叠加演示 ===
        // Coroutine 计时器句柄（防重复触发叠多个 toast）。
        Coroutine _tipsRoutine;

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[Showcase] 无 LoomStage"); return; }
        }

        // #1a1d2e = .root 背景色（showcase 深蓝底）。主相机配同色，letterbox 与 root 无缝。
        static readonly Color RootBg = new Color(26f / 255f, 29f / 255f, 46f / 255f, 1f);

        void Start()
        {
            if (_stage == null) return;
            ConfigureCameraBackground();

            // layer 骨架：root + ui_layer（主界面层）+ tips_layer（tips 层，在上）。
            _root = _stage.CreateRoot("div", "width:1080px;height:1920px;background-color:#1a1d2e;flex-direction:column");
            _uiLayer = _stage.CreateNode("div", "flex-grow:1");
            _tipsLayer = _stage.CreateNode("div", "flex-direction:column;align-items:center;justify-content:flex-end;padding:40px;pointer-events:none");
            _stage.AppendChild(_root, _uiLayer);
            _stage.AppendChild(_root, _tipsLayer);

            // load showcase 包进资源池（不建 scene）。
            byte[] pkgBytes = LoadPkgBytes(ShowcasePkgFile);
            if (pkgBytes == null)
            {
                Debug.LogError($"[Showcase] 无法加载 {ShowcasePkgFile}——showcase 不显示");
                return;
            }
            int r = _stage.LoadPackage(ShowcasePkg, pkgBytes);
            if (r != 0)
            {
                Debug.LogError($"[Showcase] LoadPackage({ShowcasePkg}) 失败 rc={r}");
                return;
            }

            OpenPage("home");
        }

        // 从 StreamingAssets 读 pkg.bin 字节。editor/player 通用（Application.streamingAssetsPath）。
        // Android 下 streamingAssetsPath 是 jar:file://... 需 UnityWebRequest；本 showcase 只跑 editor/standalone，File.ReadAllBytes 即可。
        byte[] LoadPkgBytes(string fileName)
        {
            string path = System.IO.Path.Combine(Application.streamingAssetsPath, fileName);
            if (!System.IO.File.Exists(path))
            {
                Debug.LogError($"[Showcase] pkg.bin 不存在：{path}（用 LoomPackageManagerWindow 打包）");
                return null;
            }
            return System.IO.File.ReadAllBytes(path);
        }

        // 主相机默认 Skybox；root shrink-to-fit + safeArea letterbox 后透出 → 整体灰蒙蒙。
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

        // === 导航跳页（模型 2 web 式，§7.3）===
        // OpenPage: 清当前页 listener + RemoveNode(当前页) + Instantiate(目标页) + 挂 ui_layer + SubscribePage。
        // home 也是页（ui_layer 整层换）。各页 back-home → OpenPage("home")。
        void OpenPage(string page)
        {
            if (_currentPage != uint.MaxValue)
            {
                ClearPageListeners();              // 按页清 listener（§7.5）
                _stage.RemoveNode(_currentPage);   // 摘当前页（联动清 anim/scroll/tween/focused_node）
                _currentPage = uint.MaxValue;
            }
            // 切页时若 mail 叠加层还挂着，一并摘（mail 属于 page_dyntree 的演示，切走 dyntree 页时清理）。
            if (_mailOverlay != uint.MaxValue)
            {
                _stage.RemoveNode(_mailOverlay);
                _mailOverlay = uint.MaxValue;
            }
            uint node = _stage.Instantiate(ShowcasePkg, page);
            if (node == uint.MaxValue)
            {
                Debug.LogError($"[Showcase] Instantiate({ShowcasePkg}, {page}) 失败");
                return;
            }
            _currentPage = node;
            _stage.AppendChild(_uiLayer, node);
            SubscribePage(page);
            Debug.Log($"[Showcase] OpenPage({page}) → node={node}");
        }

        // === 按页 listener 注册表（§7.5）===
        // AddPageListener: AddListener + 记进 _pageListeners（切页前 ClearPageListeners 批量 RemoveListener）。
        void AddPageListener(uint node, EventType type, EventCallback cb)
        {
            if (node == uint.MaxValue) return;
            _stage.EventHandler.AddListener(node, type, cb);
            if (!_pageListeners.TryGetValue(node, out var list))
                _pageListeners[node] = list = new List<(EventType, EventCallback)>();
            list.Add((type, cb));
        }

        // ClearPageListeners: 遍历当前页注册的 listener 逐个 RemoveListener（不用 EventHandler.Clear 粗清）。
        // 切页前调（OpenPage 里 RemoveNode 之前）。RemoveNode 后旧 NodeId 失效，listener 成悬空条目——故须先清。
        void ClearPageListeners()
        {
            foreach (var kv in _pageListeners)
            {
                uint node = kv.Key;
                foreach (var (type, cb) in kv.Value)
                    _stage.EventHandler.RemoveListener(node, type, cb);
            }
            _pageListeners.Clear();
        }

        // === 按页订阅（SubscribePage）===
        // switch page → 调对应 SubscribeXxx（每页一组，复用旧 driver 的 lamp/tween/dyntree 逻辑）。
        // 每订阅走 AddPageListener（记进注册表），切页时批量清。
        void SubscribePage(string page)
        {
            switch (page)
            {
                case "home": SubscribeHome(); break;
                case "page_controls": SubscribeControls(); break;
                case "page_text": SubscribeText(); break;
                case "page_image": SubscribeImage(); break;
                case "page_scroll": SubscribeScroll(); break;
                case "page_tween": SubscribeTween(); break;
                case "page_interact": SubscribeInteract(); break;
                case "page_dyntree": SubscribeDynTree(); break;
            }
        }

        // home：订阅各 nav 按钮 → OpenPage(目标页) + nav-tips-demo → ShowTips。
        // nav-* id 与 home.html 一致（nav-controls/nav-text/nav-image/nav-scroll/nav-tween/nav-interact/nav-dyntree/nav-tips-demo）。
        void SubscribeHome()
        {
            AddNavListener("nav-controls", "page_controls");
            AddNavListener("nav-text", "page_text");
            AddNavListener("nav-image", "page_image");
            AddNavListener("nav-scroll", "page_scroll");
            AddNavListener("nav-tween", "page_tween");
            AddNavListener("nav-interact", "page_interact");
            AddNavListener("nav-dyntree", "page_dyntree");
            // nav-tips-demo → 弹 tips_toast 演示（tips_layer 叠加）。
            uint tipsBtn = _stage.FindNodeById("nav-tips-demo");
            AddPageListener(tipsBtn, EventType.Click, _ => ShowTips());
            Debug.Log("[Showcase] home 订阅完成（7 nav + tips-demo）");
        }

        void AddNavListener(string navId, string targetPage)
        {
            uint n = _stage.FindNodeById(navId);
            AddPageListener(n, EventType.Click, _ => OpenPage(targetPage));
        }

        // 各页通用：back-home 按钮 → OpenPage("home")。
        // 所有 page_*.html 都有 id="back-home"（T10 复用语义 id）。
        void SubscribeBackHome()
        {
            uint back = _stage.FindNodeById("back-home");
            AddPageListener(back, EventType.Click, _ => OpenPage("home"));
        }

        // page_controls：back-home + btn-demo-disabled 禁用 + model-slot NativeHost 绑定。
        void SubscribeControls()
        {
            SubscribeBackHome();
            uint dbd = _stage.FindNodeById("btn-demo-disabled");
            if (dbd != uint.MaxValue) _stage.SetNodeDisabled(dbd, true);
            // NativeHost：绑外部 GO 到 model-slot（每帧 Sync 自动同步 wrapper TRS）。
            if (_nativeModel != null)
            {
                _stage.BindNativeHost("model-slot", _nativeModel);
                _nativeModel.transform.localScale = _nativeScale;
            }
            Debug.Log("[Showcase] page_controls 订阅完成（back + disabled + NativeHost）");
        }

        // page_text：back-home（无其他交互元素，纯展示文本样式）。
        void SubscribeText()
        {
            SubscribeBackHome();
            Debug.Log("[Showcase] page_text 订阅完成（back）");
        }

        // page_image：back-home（无其他交互元素，纯展示视觉样式）。
        void SubscribeImage()
        {
            SubscribeBackHome();
            Debug.Log("[Showcase] page_image 订阅完成（back）");
        }

        // page_scroll：back-home（外层 page-scroll 自带滚动行为，无需 driver 订阅）。
        void SubscribeScroll()
        {
            SubscribeBackHome();
            Debug.Log("[Showcase] page_scroll 订阅完成（back）");
        }

        // page_interact（§4 灯阵）：back-home + 各交互元素事件 + disabled + 路由。
        // 复用旧 driver 的 SubscribeLampEvents 逻辑，改用 AddPageListener（记进注册表）。
        void SubscribeInteract()
        {
            SubscribeBackHome();
            SubscribeLamp("hit-click", EventType.Click, OnClickHit);
            SubscribeLamp("hit-hover", EventType.RollOver, OnHoverHit);
            SubscribeLamp("hit-hover", EventType.RollOut, OnHoverLeave);
            SubscribeLamp("hit-drag", EventType.DragMove, OnDragHit);
            SubscribeLamp("hit-longpress", EventType.LongPress, OnLongHit);
            SubscribeLamp("hit-key", EventType.KeyDown, OnKeyHit);
            uint dn = _stage.FindNodeById("hit-disabled");
            if (dn != uint.MaxValue) _stage.SetNodeDisabled(dn, true);
            // 路由：outer/inner 均订阅 Click；inner 调 StopPropagation 止冒泡（outer 不触发）。
            SubscribeLamp("route-outer", EventType.Click, OnRouteOuter);
            SubscribeLamp("route-inner", EventType.Click, OnRouteInner);
            SubscribeLamp("route-pe", EventType.Click, OnRoutePe);
            Debug.Log("[Showcase] page_interact 灯阵订阅完成（click/hover/drag/longpress/key + route + disabled）");
        }

        void SubscribeLamp(string id, EventType t, EventCallback cb)
        {
            uint n = _stage.FindNodeById(id);
            AddPageListener(n, t, cb);
        }

        // 点亮 lamp-{name} 容器：无 get_children API，改用整容器 opacity 脉冲指示触发。
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
            if (ctx.isDoubleClick) LightLamp("click", ++_clickCount);
        }
        void OnHoverHit(EventContext ctx) { LightLamp("hover", ++_hoverCount); }
        void OnHoverLeave(EventContext ctx) { LightLamp("hover", ++_hoverCount); }
        void OnDragHit(EventContext ctx) { LightLamp("drag", ++_dragCount); }
        void OnLongHit(EventContext ctx) { LightLamp("longpress", ++_longCount); }
        void OnKeyHit(EventContext ctx) { LightLamp("key", ++_keyCount); }

        // 路由演示：inner StopPropagation → outer 不收。独立 lamp-route 反馈。
        void OnRouteOuter(EventContext ctx) { LightLamp("route", ++_routeCount); }
        void OnRouteInner(EventContext ctx)
        {
            ctx.StopPropagation();
            LightLamp("route", ++_routeCount);
        }
        void OnRoutePe(EventContext ctx) { LightLamp("route", ++_routeCount); }

        // page_tween（§7 动效）：back-home + tween 播放/kill/clear + complete 回调 + kill-target 旋转。
        // 复用旧 driver 的 SubscribeTweenDemos 逻辑，改用 AddPageListener。
        void SubscribeTween()
        {
            SubscribeBackHome();
            SubscribeLamp("tween-play", EventType.Click, OnTweenPlay);
            SubscribeLamp("ease-play", EventType.Click, OnEasePlay);
            SubscribeLamp("delay-play", EventType.Click, OnDelayPlay);
            SubscribeLamp("complete-play", EventType.Click, OnCompletePlay);
            SubscribeLamp("kill-btn", EventType.Click, OnKill);
            SubscribeLamp("clear-btn", EventType.Click, OnClear);
            // t-opacity 的 TweenComplete（core 完成时直派，ctx.clickCount=prop、ctx.touchId=tag）。
            SubscribeLamp("t-opacity", EventType.TweenComplete, OnTweenCompleteTag);
            // kill-target：启动即开始持续旋转（单次长 tween——loop 需 TweenComplete 重启，简化省略）。
            PlayProp("kill-target", TweenProp.Rotation, new float[] { 0f, 0, 0, 0 }, new float[] { 360f, 0, 0, 0 }, 4f, Ease.Linear, 0f, 0);
            Debug.Log("[Showcase] page_tween 订阅完成（play/ease/delay/complete/kill/clear + kill-target 旋转）");
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
            int[] pick = { 1, 5, 9 };
            for (int i = 0; i < pick.Length; i++)
                PlayProp("ease-" + i, TweenProp.Translate, new float[] { 0f, 0, 0, 0 }, new float[] { 200f, 0, 0, 0 }, 1.0f, _allEase[pick[i]], 0f, 0);
        }

        // delay 错峰：三块依次起。
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
            if (ctx.touchId == TagComplete) LightLamp("complete", 1);
        }

        // kill 冻结当前角（停在末值）；clear 清所有 anim 回 CSS 初始。
        void OnKill(EventContext ctx) { _stage.KillTween(_stage.FindNodeById("kill-target"), TweenProp.Rotation); }
        void OnClear(EventContext ctx) { _stage.ClearAnim(_stage.FindNodeById("kill-target")); }

        // page_dyntree（§3.10）：back-home + 建/删/批量/set_style + dyn-load-mail/showcase。
        // 复用旧 driver 的 SubscribeDynamicTree 逻辑，改用 AddPageListener + dyn-load-* 改 instantiate/remove（非切包）。
        void SubscribeDynTree()
        {
            SubscribeBackHome();
            _dynAnchor = _stage.FindNodeById("dyn-anchor");
            SubscribeLamp("dyn-add", EventType.Click, OnDynAdd);
            SubscribeLamp("dyn-add20", EventType.Click, OnDynAdd20);
            SubscribeLamp("dyn-del", EventType.Click, OnDynDel);
            SubscribeLamp("dyn-clear", EventType.Click, OnDynClear);
            SubscribeLamp("dyn-style", EventType.Click, OnDynStyle);
            // dyn-load-mail → instantiate("showcase","mail") 挂 ui_layer（叠加，非切包）。
            // dyn-load-showcase → remove mail（摘邮件叠加）。
            SubscribeLamp("dyn-load-mail", EventType.Click, OnDynLoadMail);
            SubscribeLamp("dyn-load-showcase", EventType.Click, OnDynLoadShowcase);
            Debug.Log($"[Showcase] page_dyntree 订阅完成（anchor={_dynAnchor}）");
        }

        // 建 1 个 panel（panel+title+icon 子树）。返回 panel NodeId。
        uint CreateDynPanel()
        {
            if (_dynAnchor == uint.MaxValue) return uint.MaxValue;
            _dynSeq++;
            uint panel = _stage.CreateNode("div", "width:120px;height:90px;background:#2a2f45;border-radius:8px;flex-direction:column;gap:4px;padding:6px");
            if (panel == uint.MaxValue) return uint.MaxValue;
            _stage.AppendChild(_dynAnchor, panel);
            uint title = _stage.CreateNode("span", "font-size:14px;color:#e6e6e0");
            _stage.AppendChild(panel, title);
            _stage.SetText(title, "item-" + _dynSeq);
            uint icon = _stage.CreateNode("img", "width:40px;height:40px");
            _stage.AppendChild(panel, icon);
            _stage.SetSrc(icon, "icons/skin.png");
            return panel;
        }

        void OnDynAdd(EventContext ctx)
        {
            uint panel = CreateDynPanel();
            if (panel != uint.MaxValue) _dynPanels.Add(panel);
        }

        // 批量建 20 个（测动态建树性能 + 大量子树）。
        void OnDynAdd20(EventContext ctx)
        {
            for (int i = 0; i < 20; i++)
            {
                uint panel = CreateDynPanel();
                if (panel != uint.MaxValue) _dynPanels.Add(panel);
            }
            Debug.Log($"[Showcase] 批量建 20，anchor 下共 {_dynPanels.Count} 个");
        }

        // 删最后（remove_node 联动清子 + anim/scroll/tween）。
        void OnDynDel(EventContext ctx)
        {
            if (_dynPanels.Count == 0) return;
            uint last = _dynPanels[_dynPanels.Count - 1];
            _dynPanels.RemoveAt(_dynPanels.Count - 1);
            _stage.RemoveNode(last);
        }

        // 清空所有动态建的 panel。
        void OnDynClear(EventContext ctx)
        {
            foreach (uint p in _dynPanels) _stage.RemoveNode(p);
            _dynPanels.Clear();
        }

        // toggle 末个 panel 样式（set_style 增量改 base_style + 下帧 rematch）。
        void OnDynStyle(EventContext ctx)
        {
            if (_dynPanels.Count == 0) return;
            uint last = _dynPanels[_dynPanels.Count - 1];
            _dynStyleToggled = !_dynStyleToggled;
            _stage.SetStyle(last, _dynStyleToggled
                ? "background:#c2605a;width:160px;height:70px;border-radius:16px"
                : "background:#2a2f45;width:120px;height:90px;border-radius:8px");
        }

        // dyn-load-mail（T10 交接）：instantiate("showcase","mail") 挂 ui_layer（叠加，非切包）。
        // mail 是 showcase 包内的组件（T10 合进），不需 LoadPackage 切包。
        void OnDynLoadMail(EventContext ctx)
        {
            if (_mailOverlay != uint.MaxValue)
            {
                Debug.Log("[Showcase] mail 已挂载，忽略重复 instantiate");
                return;
            }
            uint mail = _stage.Instantiate(ShowcasePkg, "mail");
            if (mail == uint.MaxValue)
            {
                Debug.LogError("[Showcase] Instantiate mail 失败");
                return;
            }
            _mailOverlay = mail;
            _stage.AppendChild(_uiLayer, mail);
            UpdateDynLoadStatus("mail");
            Debug.Log("[Showcase] mail 叠加挂载（instantiate，非切包）");
        }

        // dyn-load-showcase：remove mail（摘邮件叠加）。
        void OnDynLoadShowcase(EventContext ctx)
        {
            if (_mailOverlay == uint.MaxValue)
            {
                Debug.Log("[Showcase] mail 未挂载，无需 remove");
                return;
            }
            _stage.RemoveNode(_mailOverlay);
            _mailOverlay = uint.MaxValue;
            UpdateDynLoadStatus("showcase");
            Debug.Log("[Showcase] mail 摘除（remove）");
        }

        void UpdateDynLoadStatus(string current)
        {
            uint status = _stage.FindNodeById("dyn-load-status");
            if (status != uint.MaxValue) _stage.SetText(status, "当前：" + current);
        }

        // === tips 叠加演示（§7.3）===
        // ShowTips: instantiate("showcase","tips_toast") → append tips_layer → Coroutine 定时 RemoveNode。
        void ShowTips()
        {
            if (_tipsRoutine != null) return;   // 防重复触发叠多个 toast
            uint toast = _stage.Instantiate(ShowcasePkg, "tips_toast");
            if (toast == uint.MaxValue)
            {
                Debug.LogError("[Showcase] Instantiate tips_toast 失败");
                return;
            }
            _stage.AppendChild(_tipsLayer, toast);
            _tipsRoutine = StartCoroutine(RemoveAfter(toast, 2.0f));
            Debug.Log("[Showcase] tips 叠加显示（2s 后摘除）");
        }

        System.Collections.IEnumerator RemoveAfter(uint node, float seconds)
        {
            yield return new WaitForSeconds(seconds);
            _stage.RemoveNode(node);
            _tipsRoutine = null;
        }

        // 0-255 RGB → 归一化 [0,1] RGBA float[4]（alpha=1）。Rust tween 直接写 anim 通道，须与 style 归一化一致。
        static float[] Rgba(int r, int g, int b) => new float[] { r / 255f, g / 255f, b / 255f, 1f };
    }
}
