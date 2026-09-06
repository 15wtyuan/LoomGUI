using System;
using UnityEngine;
using Yio;

/// PlayMode showcase 查看器：导航完全走框架自己的事件系统，无 IMGUI。
///
/// 挂在与 YioStageDriver 同一 GameObject 上。Play 后：
///   - 首页 home：点 7 张 nav-card（nav-settings / nav-mail / ...）跳对应页
///   - 各页：点顶栏 / 侧栏的「← 首页」(button#back-home) 回 home
/// 这些导航元素的 id 已在 showcase HTML 里就位（home.html 的 nav-card、各页的
/// back-home），runner 只用 Button.Clicked 订阅，不画任何 Unity GUI。
/// 订阅随切页 Dispose 自动清理（public-api §5.4），切页即换树即换订阅。
///
/// 验收 7-20/7-21 改动时各页看什么：
///   - 圆角 border (P2-A)：lab(16处)/shop(7)/home(4) 等页的边框/阴影圆角，边角不突出
///   - 真 CSS block (P1)：裸 div 子元素垂直堆叠、不被 flex-grow 拉伸
///   - Image bg-color：带 background-color 的 img 有底色
///   - TextField/Password/Search 投影：form 页三种输入框类型正确
public class ShowcaseRunner : MonoBehaviour
{
    // home 页 nav-card id → showcase 组件 stem（Instantiate 第二参）。
    // 与 showcase/showcase/home.html 的 nav-card id 一一对应。
    static readonly (string cardId, string page)[] NAV_CARDS =
    {
        ("nav-settings", "settings"),
        ("nav-inventory", "inventory"),
        ("nav-mail", "mail"),
        ("nav-shop", "shop"),
        ("nav-character", "character"),
        ("nav-form", "form"),
        ("nav-lab", "lab"),
        ("nav-anim", "m2-animation"),
        ("nav-infra", "api-infra"),
        ("nav-rtcss", "runtime-css"),
        ("nav-comp", "component-lab"),
        ("nav-shape", "shape-mask"),
        ("nav-evict", "texture-lab"),
    };

    // settings 页 tab → panel 配对（HTML 标准 role=tab/tabpanel 模式）。
    // 浏览器里 yio-preview.js 的 JS 切 panel display；Yio 运行时无 JS，这里复刻该逻辑。
    // panel-audio 默认可见，其余 HTML 里 style="display:none" 冻结进 pkg。
    static readonly (string tabId, string panelId)[] SETTINGS_TABS =
    {
        ("tab-audio", "panel-audio"),
        ("tab-graphics", "panel-graphics"),
        ("tab-controls", "panel-controls"),
        ("tab-account", "panel-account"),
        ("tab-search", "panel-search"),
    };

    YioStageDriver _driver;
    Container _current;
    string _shown;

    // ── character 页 3D 展位（NativeHost 同屏渲染验证） ──
    Container _nativeSlot;         // 绑定目标（native-slot div；Unbind 需同节点）
    GameObject _characterModel;    // NativeHost 持位根（挂 wrapper 下）
    Transform _figureSpin;         // 旋转体（模型本体）
    const float FigureSpinDegPerSec = 40f;

    void Update()
    {
        if (_figureSpin != null)
            _figureSpin.Rotate(Vector3.up, FigureSpinDegPerSec * Time.deltaTime, Space.Self);
    }

    void Start()
    {
        // 编辑器验收防冻：编辑器窗口失焦（看 Console/切窗）时播放器循环会被挂起，
        // 表现为「游戏只剩一两帧」。Run In Background 让循环失焦持续跑（真机默认行为）。
        Application.runInBackground = true;
        _driver = GetComponent<YioStageDriver>();
        if (_driver == null)
        {
            Debug.LogError("[Showcase] YioStageDriver not found on same GameObject — runner wired wrong");
            return;
        }
        // 手型光标皮肤（消费侧注册示例）：包不内置任何皮肤（intent 1 缺省 = 系统箭头），
        // 这里注册像素画手型让 pressable 悬停有 affordance。热点 = 食指尖 (12,1)，
        // SetCursor 热点从纹理左上角量（Unity docs），与像素画屏幕坐标同系。
        _driver.SetCursorTexture(1u, BuildPixelHandCursorTexture(), new Vector2(12f, 1f));
        // 组件类绑定（#20）：注册须在 instantiate 前（setup 期）——晚注册只影响未来构造。
        // 显式工厂委托 = AOT 零反射（IL2CPP 安全）。
        _driver.Context.RegisterComponent("lifecycle-widget",
            (c, id) => new LifecycleWidget(c, id));
        // 让 driver Awake 完成（同帧 Awake 先于 Start，理论已就绪）+ 给 LateUpdate 几帧余量。
        Invoke(nameof(Boot), 0.1f);
    }

    void Boot()
    {
        if (_current == null) Show("home");
    }

    void Show(string page)
    {
        if (_shown == page) return;
        TeardownCharacterStage();   // 上一页若是 character：解绑 NativeHost + 销毁模型
        TeardownRuntimeCssPage();   // 上一页若是 runtime-css：Dispose 注入句柄
        TeardownComponentLabPage(); // 上一页若是 component-lab：解生命周期读数刷新事件
        if (_current != null)
        {
            _current.Dispose();   // 递归销毁旧页 + 清旧页事件订阅（Rust remove_node + 后端镜像下帧清）
            _current = null;
        }
        _current = _driver.Instantiate("showcase", page);
        _shown = _current != null ? page : null;
        if (_current == null)
        {
            Debug.Log($"[Showcase] Instantiate showcase/{page} = FAIL (pkg not loaded? comp not found?)");
            return;
        }
        WireNav(_current, page);
        WireControls(_current, page);
        WireSettingsTabs(_current, page);
        WireListViews(_current, page);
        WireCharacterStage(_current, page);
        Debug.Log($"[Showcase] Instantiate showcase/{page} = OK");
    }

    /// 用框架事件系统接导航：nav-card 与 back-home 都是 `<button>`（Button.Clicked）。
    /// （nav-card 原为 `<a>`/Link.Activated，围栏紧缩 a→button 后统一走 Button.Clicked。）
    /// TryGet 找不到（本页没该元素）就跳过——home 页无 back-home，其他页无 nav-card，各取所需。
    /// 闭包捕获的 page/target 是 per-iteration 局部，每次 Show 重新订阅当前页实例。
    void WireNav(Container page, string pageName)
    {
        // back-home 两处形态：settings 侧栏（页面域直 Get）；其余 6 页在 <page-top> 组件内
        //（打包期展开 + 硬墙作用域——组件内 id 须经 host 两跳，L3 查找边界）。
        if (!page.TryGet<Button>("back-home", out var back)
            && page.TryGet<CustomElement>("page-top", out var top))
        {
            top.TryGet<Button>("back-home", out back);
        }
        if (back != null)
            back.Clicked += () => Show("home");
        if (pageName == "m2-animation" && page.TryGet<Button>("btn-replay", out var replay))
            replay.Clicked += ReplayCurrentPage;
        if (pageName == "m2-animation")
            WireM2AnimationDrivers(page);
        if (pageName == "api-infra")
            WireInfraDrivers(page);
        if (pageName == "home")
        {
            foreach (var (cardId, target) in NAV_CARDS)
            {
                string p = target;   // 防御性局部拷贝，确保每个闭包绑各自的页名
                if (page.TryGet<Button>(cardId, out var card))
                    card.Clicked += () => Show(p);
            }
        }
    }

    /// m2-animation #11/#12：程序化动画（node.Play + 句柄 L3）的 driver 接线。
    /// #11 点盒子 Play（OnKey/OnHook 回调进 Console）；#12 按钮排控制同一句柄的
    /// Pause/Resume/Stop/Time seek。Play 每次新建 programmatic player（句柄换新）。
    void WireM2AnimationDrivers(Container page)
    {
        if (page.TryGet<Container>("b11-target", out var playTarget))
        {
            playTarget.On<ClickEvent>(_ =>
                playTarget.Play("m2-play-fade")
                    .OnEnd(() => Debug.Log("[Showcase] m2 #11 Play(m2-play-fade) end")));
        }
        if (page.TryGet<Container>("b11-hook", out var hookTarget))
        {
            hookTarget.On<ClickEvent>(_ =>
                hookTarget.Play("m2-hookanim")
                    .OnKey(0.5f, () => Debug.Log("[Showcase] m2 #11 OnKey(0.5) fired"))
                    .OnHook("half", () => Debug.Log("[Showcase] m2 #11 OnHook(half) fired")));
        }
        if (!page.TryGet<Container>("b12-target", out var handleTarget))
            return;
        Yio.AnimationHandle handle = null;
        if (page.TryGet<Button>("btn-h-play", out var bPlay))
            bPlay.Clicked += () =>
            {
                handle = handleTarget.Play("m2-play-fade");
                Debug.Log("[Showcase] m2 #12 Play -> new handle");
            };
        if (page.TryGet<Button>("btn-h-pause", out var bPause))
            bPause.Clicked += () =>
            {
                handle?.Pause();
                Debug.Log($"[Showcase] m2 #12 Pause @ t={(handle?.Time ?? -1f):F2}s");
            };
        if (page.TryGet<Button>("btn-h-resume", out var bResume))
            bResume.Clicked += () =>
            {
                handle?.Resume();
                Debug.Log("[Showcase] m2 #12 Resume");
            };
        if (page.TryGet<Button>("btn-h-stop", out var bStop))
            bStop.Clicked += () =>
            {
                handle?.Stop();
                Debug.Log("[Showcase] m2 #12 Stop（句柄失效）");
            };
        if (page.TryGet<Button>("btn-h-seek", out var bSeek))
            bSeek.Clicked += () =>
            {
                if (handle == null) return;
                handle.Time = 0.5f;
                Debug.Log("[Showcase] m2 #12 seek Time=0.5s");
            };
    }

    /// api-infra 页：公共 API 基础设施验收 driver（调度三件套 / option-tab 派生 getter /
    /// 多模板列表 / 包生命周期）。页面只到 HTML 结构，行为全部在此接线——真机上看的就是这些
    /// （确定性断言在 headless SchedulerAndLifecycleTests，本页是行为/视觉面）。
    /// 切页防御：CallLater/CallNextFrame 挂在 UIContext 上跨页存活，回调里对目标节点
    /// IsDisposed 短路；OnUpdate 订阅随页 Dispose 自动清理（契约），无需手动拆。
    void WireInfraDrivers(Container page)
    {
        var ui = _driver.Context;

        // ── #1 OnUpdate 逻辑时钟：dt 累积逐帧刷新 + 帧计数；按钮 Dispose / 重订阅句柄。 ──
        // span 打包后是 TextElement（SemanticKind::TextElement；运行时 create_node("span")
        // 才产 TextNode——两路径不同型，TryGet 按 C# 类型精确匹配，写错型整块静默跳过）。
        if (page.TryGet<TextElement>("infra-clock", out var clock) &&
            page.TryGet<TextElement>("infra-frames", out var frames))
        {
            float elapsed = 0f;
            long pumps = 0;
            void Tick(float dt)
            {
                elapsed += dt;
                pumps++;
                clock.TextContent = elapsed.ToString("F1") + " s";
                frames.TextContent = pumps + " 帧";
            }
            var sub = page.OnUpdate(Tick);
            if (page.TryGet<Button>("btn-clock-toggle", out var toggle))
                toggle.Clicked += () =>
                {
                    if (sub == null) sub = page.OnUpdate(Tick);
                    else { sub.Dispose(); sub = null; }
                };
        }

        // ── #2 CallLater 倒计时链：每步 1s 延迟，one-shot 链式调度。 ──
        if (page.TryGet<Button>("btn-later", out var laterBtn) &&
            page.TryGet<TextElement>("infra-later", out var later))
        {
            laterBtn.Clicked += () =>
            {
                later.Classes.Remove("done");
                InfraCountdown(later, ui, 3);
            };
        }

        // ── #3 CallNextFrame：点击当帧「已受理」，下一帧帧头改文本。 ──
        if (page.TryGet<Button>("btn-nf", out var nfBtn) &&
            page.TryGet<TextElement>("infra-nf", out var nf) &&
            page.TryGet<TextElement>("infra-nf-count", out var nfCount))
        {
            int fired = 0;
            nfBtn.Clicked += () =>
            {
                nf.TextContent = "已点击（本帧受理）→ 等待下一帧…";
                ui.CallNextFrame(() =>
                {
                    if (nf.IsDisposed) return;
                    fired++;
                    nf.TextContent = "下一帧回调已触发 ✓（帧头 fire）";
                    nfCount.TextContent = fired + " 次";
                });
            };
        }

        // ── #4 Dropdown value 链读数：SelectedValue / option.Value / option.Selected。 ──
        if (page.TryGet<Dropdown>("infra-dd", out var dd) &&
            page.TryGet<TextElement>("dd-sel", out var ddSel) &&
            page.TryGet<TextElement>("dd-va", out var va) && page.TryGet<TextElement>("dd-sa", out var sa) &&
            page.TryGet<TextElement>("dd-vb", out var vb) && page.TryGet<TextElement>("dd-sb", out var sb) &&
            page.TryGet<TextElement>("dd-vc", out var vc) && page.TryGet<TextElement>("dd-sc", out var sc) &&
            page.TryGet<OptionItem>("opt-lang-a", out var oa) &&
            page.TryGet<OptionItem>("opt-lang-b", out var ob) &&
            page.TryGet<OptionItem>("opt-lang-c", out var oc))
        {
            void RefreshDd()
            {
                ddSel.TextContent = dd.SelectedValue ?? "(null)";
                va.TextContent = oa.Value; sa.TextContent = oa.Selected ? "true" : "false";
                vb.TextContent = ob.Value; sb.TextContent = ob.Selected ? "true" : "false";
                vc.TextContent = oc.Value; sc.TextContent = oc.Selected ? "true" : "false";
            }
            RefreshDd();
            dd.SelectionChanged += _ => RefreshDd();
        }

        // ── #5 Tab.Selected 合成读数：切选即跟随（父 TabList 状态派生）。 ──
        if (page.TryGet<TabList>("infra-tabs", out var tabs) &&
            page.TryGet<Tab>("itab-1", out var t1) && page.TryGet<Tab>("itab-2", out var t2) &&
            page.TryGet<Tab>("itab-3", out var t3) &&
            page.TryGet<TextElement>("tab-r1", out var r1) &&
            page.TryGet<TextElement>("tab-r2", out var r2) &&
            page.TryGet<TextElement>("tab-r3", out var r3))
        {
            void RefreshTabs()
            {
                r1.TextContent = t1.Selected ? "true" : "false";
                r2.TextContent = t2.Selected ? "true" : "false";
                r3.TextContent = t3.Selected ? "true" : "false";
            }
            RefreshTabs();
            tabs.SelectionChanged += _ => RefreshTabs();
        }

        // ── #6 多模板 ListView：GetTemplate 取两个蓝图 + TemplateSelector 按 index 分派。
        //    强调行视觉（金左条 + 64px）烙在 row-tpl-accent 蓝图里——BindItem 只填数据。
        //    动态面四按钮（读数翻转即证据，照 #7 同款）：
        //    插/删走 Notify*（C# 侧按新 index 重推受影响区间，模板随数据重排）；
        //    「切换全强调」换 selector（core park 旧蓝图 slot、下帧以新蓝图重新物化——整列翻面）；
        //    「null selector 试验」验证严格派异常（求值抛在前、core 映射未动，捕获后恢复原状）。
        if (page.TryGet<ListView>("infra-mt-list", out var mt) &&
            page.TryGet<TextElement>("infra-mt-status", out var mtStatus))
        {
            UITemplate rowNormal = mt.GetTemplate("row-tpl");
            UITemplate rowAccent = mt.GetTemplate("row-tpl-accent");
            System.Func<int, UITemplate> alternating = i => (i % 3 == 2) ? rowAccent : rowNormal;
            bool allAccent = false;
            mt.TemplateSelector = alternating;
            mt.BindItem = (item, i) =>
            {
                var spans = item.Query<TextElement>();
                if (spans.Count >= 2)
                {
                    spans[0].TextContent = string.Format("#{0:00}", i);
                    spans[1].TextContent = (i % 3 == 2) ? "强调行（蓝图切换）" : "普通行";
                }
            };
            mt.ItemCount = 30;
            if (page.TryGet<Button>("btn-mt-insert", out var mtInsert))
                mtInsert.Clicked += () =>
                {
                    mt.NotifyInserted(0, 3);
                    mtStatus.TextContent = "已插 3 @0 · 共 " + mt.ItemCount + " 项 · 模板按新 index 重排";
                };
            if (page.TryGet<Button>("btn-mt-remove", out var mtRemove))
                mtRemove.Clicked += () =>
                {
                    if (mt.ItemCount < 6) { mtStatus.TextContent = "项数不足，先插几批再删"; return; }
                    mt.NotifyRemoved(0, 3);
                    mtStatus.TextContent = "已删 [0,3) · 共 " + mt.ItemCount + " 项";
                };
            if (page.TryGet<Button>("btn-mt-allaccent", out var mtAll))
                mtAll.Clicked += () =>
                {
                    allAccent = !allAccent;
                    mt.TemplateSelector = allAccent ? (System.Func<int, UITemplate>)(_ => rowAccent) : alternating;
                    mtStatus.TextContent = (allAccent ? "全强调蓝图 ✓（换 selector 重物化）" : "恢复交替分派 ✓") + " · 共 " + mt.ItemCount + " 项";
                };
            if (page.TryGet<Button>("btn-mt-nullsel", out var mtNull))
                mtNull.Clicked += () =>
                {
                    // 严格派探针：setter 赋值即对 [0, ItemCount) 全量求值——null 探针必须落在
                    // 当前范围内，删项后固定 @5 会探空（求值全非 null → 不抛），误报「未抛」。
                    if (mt.ItemCount == 0) { mtStatus.TextContent = "共 0 项——无 index 可探"; return; }
                    int probe = System.Math.Min(5, mt.ItemCount - 1);
                    var prev = mt.TemplateSelector;
                    try
                    {
                        // 求值抛在推送之前——core 的 per-item 映射未被动过，列表原样。
                        // setter 先落 C# 字段后校验：捕获后必须显式回设，getter 才不与 core 脱钩。
                        mt.TemplateSelector = i => (i == probe) ? null : rowNormal;
                        mtStatus.TextContent = "未抛（✗ 严格派失效）";
                    }
                    catch (UIContractException)
                    {
                        mtStatus.TextContent = "null @" + probe + " 抛 UIContractException ✓ · 已恢复原状 · " + mt.ItemCount + " 项不变";
                    }
                    finally
                    {
                        // 恢复点击前状态——全强调开着时恢复交替会让 allAccent 与实际 selector 脱钩。
                        mt.TemplateSelector = prev;
                    }
                };
        }

        // ── #7 UnloadPackage：别名重载 showcase 字节 → 实例化本页微缩窗 → 卸载见存活。
        //    载荷用 api-infra 组件本身（pkg 可寻址组件粒度 = html 文件，无需另造载荷组件）：
        //    实例后 Style 覆写宽高 + overflow hidden 裁成小窗。
        if (page.TryGet<Container>("infra-ul-stage", out var ulStage) &&
            page.TryGet<TextElement>("infra-ul-status", out var ulStatus))
        {
            UIPackage copyPkg = null;
            const string CopyName = "infra-copy";
            Container InstantiateMiniWindow()
            {
                var win = copyPkg.Instantiate("api-infra");
                win.Style.Width = Length.Px(420);
                win.Style.Height = Length.Px(88);
                win.Style.OverflowX = Overflow.Clip;
                win.Style.OverflowY = Overflow.Clip;
                // 微缩窗 88px 高只露出顶栏，正文整棵被 clip 不可见——却照付全额 solve 成本
                //（solve 每帧全量重建 taffy 树，每 mini ≈ +7.7ms/帧，几个窗口就把帧率拖垮）。
                // display:none 让 solve 跳过该子树（taffy 语义），视觉零变化。
                foreach (var c in win.Query<Container>())
                {
                    if (c.Classes.Contains("body")) { c.Style.Display = DisplayMode.None; break; }
                }
                ulStage.AddChild(win);
                return win;
            }
            if (page.TryGet<Button>("btn-ul-load", out var ulLoad))
                ulLoad.Clicked += () =>
                {
                    try
                    {
                        if (copyPkg == null)
                            copyPkg = ui.LoadPackage(CopyName, _driver.LoadPackageBytes("showcase"));
                        InstantiateMiniWindow();
                        ulStatus.TextContent = "已实例化（api-infra 微缩窗 · 舞台上 " + ulStage.ChildCount + " 个存活）";
                    }
                    catch (System.Exception ex)
                    {
                        ulStatus.TextContent = "Load/Instantiate 异常：" + ex.GetType().Name;
                    }
                };
            if (page.TryGet<Button>("btn-ul-unload", out var ulUnload))
                ulUnload.Clicked += () =>
                {
                    if (copyPkg == null) { ulStatus.TextContent = "副本未加载"; return; }
                    try
                    {
                        ui.UnloadPackage(CopyName);
                        bool staleThrew = false;
                        try { copyPkg.Instantiate("api-infra"); }
                        catch (UIPackageException) { staleThrew = true; }
                        ulStatus.TextContent = "模板已卸载 · 旧句柄 Instantiate 抛 = " + (staleThrew ? "✓" : "✗")
                            + " · 微缩窗独立存活 = " + (ulStage.ChildCount > 0 ? "✓" : "✗");
                        copyPkg = null;   // 下次 Load 重建句柄（重载同名包）
                    }
                    catch (System.Exception ex)
                    {
                        ulStatus.TextContent = "Unload 异常：" + ex.GetType().Name;
                    }
                };
        }
    }

    /// #2 的倒计时链：n→…→1→完成，每步 CallLater(1s)（递归延迟调度）。切页后目标节点
    /// 已 Dispose 即短路（timer 挂在 UIContext 上跨页存活，不随页清理）。
    void InfraCountdown(TextElement label, UIContext ui, int n)
    {
        if (label.IsDisposed) return;
        if (n == 0)
        {
            label.TextContent = "完成 ✓";
            label.Classes.Add("done");
            return;
        }
        label.TextContent = n.ToString();
        ui.CallLater(1f, () => InfraCountdown(label, ui, n - 1));
    }

    /// m2-animation 页「↻ 重播」：原地重启声明式动画（Container.RestartAnimations）——
    /// player 重建、delay 重计，节点/滚动/控件值/订阅全保留。
    void ReplayCurrentPage()
    {
        // 原地重启声明式动画（Container.RestartAnimations）：player 重建、delay 重计，
        // 节点/滚动/控件值/订阅全保留——不再走销毁重实例化。
        _current?.RestartAnimations();
    }

    // ── character 页 3D 展位：NativeHost 把引擎 GO 嵌进 UI 层级 ──
    //
    // 验证目标：UI（自绘 mesh）与引擎原生渲染（3D 模型 + 光照）同屏 interleaved——
    // 模型 sortingOrder = native-slot 节点 sort_key（NativeHostManager.Sync 每帧写），
    // 与 UI 同 Transparent 队列按 UI 绘制序穿插；模型跟随节点 world transform。
    // 模型用基元拼装（无外部资产依赖）：机甲 + 剑 + 自发光基座 + 点光（ shading 证明
    // 走的是引擎光照而非 UI 自绘）。尺寸按 design px：holder scale 100 → 1 unit = 100px。

    void WireCharacterStage(Container page, string pageName)
    {
        if (pageName != "character") return;
        if (!page.TryGet<Container>("native-slot", out var slot)) return;
        _nativeSlot = slot;
        _characterModel = BuildCharacterModel(out _figureSpin);
        _driver.BindNativeHost(slot, _characterModel);
        // 帧延迟对齐：build 时（Animator 未评估）的 bounds 与真实播放 pose 有偏差（曾整体高出
        // 展位数百 px）——等动画跑 2 帧后按世界包围盒重新归一：高 520、中心对齐展位中心。
        StartCoroutine(AlignModelAfterAnimEval(_characterModel.transform));
        Debug.Log("[Showcase] character native-slot bound to 3D model (NativeHost)");
    }

    /// 帧延迟对齐：build 时（Animator 未评估）的 bounds 与真实播放 pose 有偏差——模型
    /// 曾整体高出展位数百万至更多 px（脚底钉在展位中心、身高向上溢出展位顶）。
    /// 等 2 帧动画真实评估后按世界包围盒重新归一：高 520、中心对齐展位中心（持位原点）。
    System.Collections.IEnumerator AlignModelAfterAnimEval(Transform modelRoot)
    {
        yield return null;
        yield return null;
        var rends = modelRoot.GetComponentsInChildren<Renderer>();
        if (rends.Length == 0) yield break;
        var b = rends[0].bounds;
        foreach (var r in rends) b.Encapsulate(r.bounds);
        if (b.size.y < 0.001f || b.size.y > 10000f) yield break;
        float s = 520f / b.size.y;
        modelRoot.localScale *= s;
        // 缩放后包围盒随 localScale 变化——重测一次再对齐（两步收敛）。
        b = rends[0].bounds;
        foreach (var r in rends) b.Encapsulate(r.bounds);
        // 中心对齐到 modelRoot（holder）自身位置 = 展位中心（不是 wrapper 原点 = slot 左上）。
        Vector3 worldOffset = modelRoot.position - b.center;
        modelRoot.position += worldOffset;
        // 观察向（z）压扁 + 抬到 UI 平面前：模型原生 z 深 ~±135px，超出 UI 相机视景
        // （near z=-9.9 / far z=90）会被远近裁剪面各切一刀（视觉"被 UI 平面切成两半，
        // 只剩后半"）。holder（不随自转）压 z 至 ~1/4 并整体 z+=20 → z∈[20..87]，
        // 全程在裁剪区间内、位于 UI 平面（z=0）之前。
        Vector3 ls = modelRoot.localScale;
        modelRoot.localScale = new Vector3(ls.x, ls.y, ls.z * 0.25f);
        Vector3 pos = modelRoot.position;
        modelRoot.position = new Vector3(pos.x, pos.y, pos.z + 20f);
        Debug.Log($"[Showcase] model aligned: size={b.size} center={b.center} rootPos={modelRoot.position}");
    }

    void TeardownCharacterStage()
    {
        if (_nativeSlot != null)
        {
            _driver.UnbindNativeHost(_nativeSlot);   // 销毁 wrapper（GO 先 reparent 出来）
            _nativeSlot = null;
        }
        if (_characterModel != null)
        {
            Destroy(_characterModel);
            _characterModel = null;
        }
        _figureSpin = null;
    }

    /// 展位模型：优先 FBX 资产（Animated Human prefab，含 Animator controller 自动播
    /// 骨骼动画——验证 NativeHost 带真实 SkinnedMeshRenderer + 动画同屏渲染）；资产缺失
    /// （built player / 路径变动）回落程序化基元机甲。两者都做归一化：骨架/渲染包围盒
    /// 缩放到 ~520 design px、脚底对齐持位点、水平居中，模型细节与资产原始尺寸解耦。
    static GameObject BuildCharacterModel(out Transform spin)
    {
#if UNITY_EDITOR
        var prefab = UnityEditor.AssetDatabase.LoadAssetAtPath<GameObject>(
            "Assets/Models/quaternius_animatedman/Animated Human.prefab");
        if (prefab != null)
        {
            // 归一化期间 holder 必须留在原点：bounds 是世界系读数，holder 若已带 slot 偏移
            // （360,-340），偏移会被当几何中心反向"归位"——模型被甩出数万单位（曾现）。
            // 量完再挪到展位中心。
            var holder = new GameObject("NativeCharacter");

            var inst = Instantiate(prefab, holder.transform);
            inst.transform.localPosition = Vector3.zero;
            inst.transform.localRotation = Quaternion.identity;
            inst.transform.localScale = Vector3.one;
            // 骨骼动画 pose 决定 skinned bounds——先评估首帧再量。骨架 AABB 一并封装
            //（蒙皮渲染的真值；SMR.bounds 在 skinning 首评估前可能是陈旧的小盒）。
            var animator = inst.GetComponentInChildren<Animator>();
            if (animator != null)
            {
                animator.applyRootMotion = false;
                animator.Rebind();
                animator.Update(0f);
            }
            var rends = inst.GetComponentsInChildren<Renderer>();
            bool have = false;
            var b = new Bounds();
            foreach (var r in rends)
            {
                if (!have) { b = r.bounds; have = true; }
                else b.Encapsulate(r.bounds);
            }
            foreach (var smr in inst.GetComponentsInChildren<SkinnedMeshRenderer>())
            {
                smr.updateWhenOffscreen = true;   // 骨架驱动世界 bounds，杜绝误剔除
                foreach (var bone in smr.bones)
                    if (bone != null)
                    {
                        if (!have) { b = new Bounds(bone.position, Vector3.zero); have = true; }
                        else b.Encapsulate(bone.position);
                    }
            }
            if (have && b.size.y > 0.001f && b.size.y < 10000f)
            {
                float s = 520f / b.size.y;
                inst.transform.localScale = Vector3.one * s;
                // 脚底对齐 + 水平/纵深居中（旋转 pivot = 脚底中心）。
                inst.transform.localPosition = new Vector3(
                    -b.center.x * s, -b.min.y * s, -b.center.z * s);
                // z 微前：与 slot 自身底色同 sort_key 时以距离赢 tiebreak（近者后画）。
                inst.transform.localPosition += new Vector3(0f, 0f, 0.5f);
            }
            // wrapper 原点 = native-slot 左上角（design 坐标 y 下 → container y-up 空间取负）。
            // slot 720x680 → 持位居中、脚底落在中心点。
            holder.transform.localPosition = new Vector3(360f, -340f, 0f);


            var lightGo = new GameObject("rimLight");
            lightGo.transform.SetParent(holder.transform, false);
            lightGo.transform.localPosition = Vector3.zero;
            // 平行光（无距离衰减；design px 尺度的模型下点光衰减到近黑）+ 暖色斜照。
            var pl = lightGo.AddComponent<Light>();
            pl.type = LightType.Directional;
            pl.transform.localRotation = Quaternion.Euler(50f, -30f, 0f);
            pl.color = new UnityEngine.Color(1f, 0.94f, 0.85f);
            pl.intensity = 2.2f;
            // 正面补光（贴图深色系，纯侧逆光太暗）：从相机方向低强度补。
            var fillGo = new GameObject("fillLight");
            fillGo.transform.SetParent(holder.transform, false);
            var fl = fillGo.AddComponent<Light>();
            fl.type = LightType.Directional;
            fl.transform.localRotation = Quaternion.Euler(10f, 190f, 0f);
            fl.color = new UnityEngine.Color(0.85f, 0.92f, 1f);
            fl.intensity = 0.9f;
            Debug.Log($"[Showcase] native-slot model = FBX prefab（Animator={animator != null}）");
            spin = inst.transform;
            return holder;
        }
        Debug.LogWarning("[Showcase] Animated Human.prefab not found — fallback to primitive mech");
#endif
        return BuildPrimitiveMech(out spin);
    }

    /// 程序化机甲（FBX 缺失时的 fallback）：躯干/头/肩/臂 capsule+cube，右手发光剑，
    /// 脚下发光基座环，一点光。
    static GameObject BuildPrimitiveMech(out Transform spin)
    {
        var holder = new GameObject("NativeCharacter");
        // wrapper 原点 = native-slot 左上角（design 坐标 y 下 → container y-up 空间取负）。
        // slot 720x680 → 持位居中。figure z +0.01：与 slot 自身底色同 sort_key 时以 z
        // 近者后画赢 tiebreak，保证模型画在底色之上。
        holder.transform.localPosition = new Vector3(360f, -340f, 0f);
        holder.transform.localScale = Vector3.one * 100f;

        var figure = new GameObject("figure");
        figure.transform.SetParent(holder.transform, false);
        figure.transform.localPosition = new Vector3(0f, 0f, 0.01f);

        var steel = new UnityEngine.Color(0.55f, 0.62f, 0.70f);
        var armor = new UnityEngine.Color(0.16f, 0.30f, 0.42f);

        Prim(figure.transform, PrimitiveType.Capsule, "torso",
            new Vector3(0f, 1.05f, 0f), new Vector3(0.55f, 0.50f, 0.42f), armor);
        Prim(figure.transform, PrimitiveType.Sphere, "head",
            new Vector3(0f, 1.74f, 0f), new Vector3(0.34f, 0.32f, 0.34f), steel);
        // 面甲：自发光青条（朝相机面 z+）。
        Prim(figure.transform, PrimitiveType.Cube, "visor",
            new Vector3(0f, 1.78f, 0.30f), new Vector3(0.26f, 0.07f, 0.05f),
            new UnityEngine.Color(0f, 0f, 0f), new UnityEngine.Color(0.37f, 0.71f, 0.83f) * 3f);
        Prim(figure.transform, PrimitiveType.Cube, "shoulderL",
            new Vector3(-0.44f, 1.42f, 0f), new Vector3(0.26f, 0.18f, 0.30f), armor);
        Prim(figure.transform, PrimitiveType.Cube, "shoulderR",
            new Vector3(0.44f, 1.42f, 0f), new Vector3(0.26f, 0.18f, 0.30f), armor);
        Prim(figure.transform, PrimitiveType.Capsule, "armL",
            new Vector3(-0.46f, 1.02f, 0f), new Vector3(0.13f, 0.32f, 0.13f), steel);
        Prim(figure.transform, PrimitiveType.Capsule, "armR",
            new Vector3(0.46f, 1.02f, 0f), new Vector3(0.13f, 0.32f, 0.13f), steel);
        // 剑：右手竖持，自发光金刃 + 小幅倾斜。
        var sword = Prim(figure.transform, PrimitiveType.Cube, "sword",
            new Vector3(0.62f, 1.25f, 0.08f), new Vector3(0.07f, 1.15f, 0.13f),
            new UnityEngine.Color(0f, 0f, 0f), new UnityEngine.Color(0.83f, 0.64f, 0.31f) * 2.5f);
        sword.transform.localRotation = Quaternion.Euler(0f, 0f, 10f);
        // 基座环：自发光青（对称体，随 figure 旋转不可见）。
        Prim(figure.transform, PrimitiveType.Cylinder, "baseRing",
            new Vector3(0f, 0.02f, 0f), new Vector3(1.0f, 0.015f, 1.0f),
            new UnityEngine.Color(0f, 0f, 0f), new UnityEngine.Color(0.37f, 0.71f, 0.83f) * 1.6f);

        var lightGo = new GameObject("rimLight");
        lightGo.transform.SetParent(figure.transform, false);
        lightGo.transform.localPosition = new Vector3(0.8f, 2.3f, 1.4f);
        var pl = lightGo.AddComponent<Light>();
        pl.type = LightType.Point;
        pl.color = new UnityEngine.Color(1f, 0.9f, 0.75f);
        pl.intensity = 1.6f;
        pl.range = 4f;

        spin = figure.transform;
        return holder;
    }

    /// 基元快捷构造：挂父、定位、缩放、赋 lit 材质（可选自发光），剥 Collider（UI 层无物理）。
    static GameObject Prim(Transform parent, PrimitiveType type, string name,
        Vector3 localPos, Vector3 localScale, UnityEngine.Color color, UnityEngine.Color? emission = null)
    {
        var go = GameObject.CreatePrimitive(type);
        go.name = name;
        var col = go.GetComponent<Collider>();
        if (col != null) Destroy(col);
        go.transform.SetParent(parent, false);
        go.transform.localPosition = localPos;
        go.transform.localScale = localScale;
        var shader = Shader.Find("Universal Render Pipeline/Lit");
        if (shader == null) shader = Shader.Find("Standard");
        var m = new Material(shader);
        m.color = color;
        if (emission.HasValue)
        {
            m.EnableKeyword("_EMISSION");
            m.SetColor("_EmissionColor", emission.Value);
        }
        go.GetComponent<Renderer>().sharedMaterial = m;
        return go;
    }

    /// settings 页 tab 切换：HTML 的 role=tab/tabpanel 模式依赖运行时 JS 改 panel display，
    /// Yio 运行时无 JS，这里订阅 tab 按钮 Clicked → 隐藏当前 panel + 显示目标 panel。
    /// panel 是裸 <div>（.panel CSS 无 display 声明）→ 默认 display:block（子元素 page-title/
    /// page-desc/field 垂直堆叠）。显示用 DisplayMode.Block，**不能用 Flex**——Flex 默认
    /// flex-direction:row 会让 panel 的子元素水平排列，布局错乱。隐藏用 DisplayMode.None。
    /// 改 Style.Display 攒批下帧 flush 到 core 触发 solve 重排（display 变是低频 UI 操作）。
    void WireSettingsTabs(Container page, string pageName)
    {
        if (pageName != "settings") return;
        // 预取 tab 按钮与 panel，过滤掉本页不存在的（宽松查询，同 WireNav）。
        var tabs = new System.Collections.Generic.List<(Button tab, Container panel)>();
        foreach (var (tabId, panelId) in SETTINGS_TABS)
        {
            if (page.TryGet<Button>(tabId, out var tab) && page.TryGet<Container>(panelId, out var panel))
                tabs.Add((tab, panel));
        }
        if (tabs.Count == 0) return;
        // 找当前可见的 panel 作初始 active（HTML 里 panel-audio 默认可见）。
        Container initial = null;
        foreach (var (_, panel) in tabs)
        {
            if (panel.Style.Display != DisplayMode.None) { initial = panel; break; }
        }
        // active 用单元素数组承载：C# 闭包捕获数组引用，所有 tab 闭包共享 arr[0]，
        // 任一 tab 点击后更新它，其余 tab 下次点击读到最新 active（避免 per-iteration 快照失同步）。
        var active = new Container[] { initial };
        foreach (var (tab, panel) in tabs)
        {
            Container target = panel;        // 防御性局部拷贝
            tab.Clicked += () =>
            {
                if (active[0] == target) return;   // 已是当前页，no-op
                if (active[0] != null) active[0].Style.Display = DisplayMode.None;
                target.Style.Display = DisplayMode.Block;
                active[0] = target;                // 后续点击以新 active 为基准
            };
        }
    }
    /// 控件事件流演示：settings 滑块拖动更新旁边数值、character 训练按钮给 EXP 进度条加经验。
    /// 只验证 ValueChanged / Clicked → ProgressBar.Value 的端到端事件链，不构建完整逻辑。
    /// 元素缺失（本页没该控件）TryGet 返 false 跳过——和 WireNav 同样的宽松查询模式。
    // runtime-css 页（#11）当前 Add 句柄集——离页全 Dispose（注入规则不跨页泄漏）。
    readonly System.Collections.Generic.List<System.IDisposable> _rtRegs = new();

    /// 离开 runtime-css 页：Dispose 全部 Add 句柄（注入规则不跨页泄漏；主题 SetVar
    /// 随页面节点销毁自然失效——node_vars 挂节点）。
    void TeardownRuntimeCssPage()
    {
        foreach (var r in _rtRegs) r?.Dispose();
        _rtRegs.Clear();
    }

    /// runtime-css 页（#11）：StyleSheet.Add/Dispose/Clear + SetVar/RemoveVar + var() 消费面。
    /// 判据（肉眼强信号）：目标块变色/复原、同优先后 Add 赢、非法 CSS 异常读数带行列、
    /// chips 组整组翻色/回落、嵌套链 swatch 变色、行内源 chip 恒橙（打包期通路回归）。
    /// 环 warning 判据不在 PlayMode（走 yio check 输出，agent 自测）。
    /// shape-mask 页（#52）：命中穿透读数（圆/角计数）+ 运行时注入圆遮罩。
    /// 判据（肉眼强信号）：点橙角=角读数翻（clip-path 裁命中——穿透到下层按钮）、
    /// 点圆心=圆读数翻、注入后方图变圆/撤销复原。静态区无接线（CSS 声明面）。
    /// 注入句柄复用 _rtRegs（离页切换统一 Dispose，同 runtime-css 生命周期）。
    void WireShapeMaskPage(Container page)
    {
        var ui = _driver.Context;
        var ss = ui.StyleSheet;
        TextElement status = null;
        page.TryGet<TextElement>("sm-status", out status);
        if (status != null) status.TextContent = "待命";
        TextElement hitC = null, hitX = null;
        page.TryGet<TextElement>("sm-hit-c", out hitC);
        page.TryGet<TextElement>("sm-hit-x", out hitX);
        if (hitC != null && hitX != null)
        {
            int c = 0, x = 0;
            Button centerBtn = null, cornerBtn = null;
            if (page.TryGet<Button>("sm-center", out centerBtn))
                centerBtn.Clicked += () => { c++; hitC.TextContent = c.ToString(); };
            if (page.TryGet<Button>("sm-corner", out cornerBtn))
                cornerBtn.Clicked += () => { x++; hitX.TextContent = x.ToString(); };
        }
        Button rtBtn = null, offBtn = null;
        if (page.TryGet<Button>("sm-rt", out rtBtn))
            rtBtn.Clicked += () =>
            {
                foreach (var r in _rtRegs) r?.Dispose();
                _rtRegs.Clear();
                _rtRegs.Add(ss.Add(".sm-rt-img { clip-path: circle(50%); }"));
                if (status != null) status.TextContent = "已注入圆";
            };
        if (page.TryGet<Button>("sm-rt-off", out offBtn))
            offBtn.Clicked += () =>
            {
                foreach (var r in _rtRegs) r?.Dispose();
                _rtRegs.Clear();
                if (status != null) status.TextContent = "已撤销";
            };
        // G 圆角命中读数：圆角裁剪器（overflow+radius）角外点击穿透（Q6 存量偏差修复）。
        TextElement rHitC = null, rHitX = null;
        page.TryGet<TextElement>("sm-rhit-c", out rHitC);
        page.TryGet<TextElement>("sm-rhit-x", out rHitX);
        if (rHitC != null && rHitX != null)
        {
            int rc = 0, rx = 0;
            Button rCenterBtn = null, rCornerBtn = null;
            if (page.TryGet<Button>("sm-r-center", out rCenterBtn))
                rCenterBtn.Clicked += () => { rc++; rHitC.TextContent = rc.ToString(); };
            if (page.TryGet<Button>("sm-r-corner", out rCornerBtn))
                rCornerBtn.Clicked += () => { rx++; rHitX.TextContent = rx.ToString(); };
        }
        // F2 var() 换形：遮罩值走 var(--sm-mask)，SetVar(string) 在圆/六边形间切。
        Image varImg = null;
        page.TryGet<Image>("sm-var-img", out varImg);
        Button varBtn = null;
        if (varImg != null && page.TryGet<Button>("sm-var", out varBtn))
        {
            bool round = false;
            varBtn.Clicked += () =>
            {
                round = !round;
                varImg.Style.SetVar(
                    "--sm-mask",
                    round ? "circle(50%)" : "polygon(25% 0%, 75% 0%, 100% 50%, 75% 100%, 25% 100%, 0% 50%)");
                if (status != null) status.TextContent = round ? "var·圆" : "var·六边";
            };
        }
    }

    void WireRuntimeCssPage(Container page)
    {
        var ui = _driver.Context;
        var ss = ui.StyleSheet;
        if (!page.TryGet<TextElement>("rt-status", out var status)) return;
        void Say(string s) { status.TextContent = s; Debug.Log($"[Showcase] rt-css: {s}"); }
        void DropRegs()
        {
            foreach (var r in _rtRegs) r?.Dispose();
            _rtRegs.Clear();
        }

        // ① Add 生效 + Dispose 复原：注入 .rt-target 红规则。
        if (page.TryGet<Button>("rt-add", out var addBtn))
            addBtn.Clicked += () =>
            {
                DropRegs();
                _rtRegs.Add(ss.Add(".rt-target { background-color: #c0392b; border-color: #c0392b; }"));
                Say("已注入红");
            };
        if (page.TryGet<Button>("rt-dispose", out var disBtn))
            disBtn.Clicked += () =>
            {
                DropRegs();
                Say("已撤销");
            };
        // ② 同 specificity 后 Add 赢：连注绿→橙两条，橙（后者）胜出；撤销后回灰。
        if (page.TryGet<Button>("rt-later", out var laterBtn))
            laterBtn.Clicked += () =>
            {
                DropRegs();
                _rtRegs.Add(ss.Add(".rt-target { background-color: #2ecc71; border-color: #2ecc71; }"));
                _rtRegs.Add(ss.Add(".rt-target { background-color: #f39c12; border-color: #f39c12; }"));
                Say("后Add赢·橙");
            };
        // ⑥ Clear 全清（pkg 规则不动——chips 边框色是 pkg 规则的 var 消费，Clear 后仍在）。
        if (page.TryGet<Button>("rt-clear", out var clearBtn))
            clearBtn.Clicked += () =>
            {
                DropRegs();
                ss.Clear();
                Say("已Clear");
            };
        // ③ 非法 CSS：at-rule 在注入通道全拒 → UIStyleException 带行列读数。
        if (page.TryGet<Button>("rt-bad", out var badBtn))
            badBtn.Clicked += () =>
            {
                try
                {
                    ss.Add("@keyframes fade { from { opacity: 0 } }");
                    Say("未抛异常?");
                }
                catch (UIStyleException ex)
                {
                    Say($"UIStyleException L{ex.Line}C{ex.Column}");
                }
            };
        // ④ SetVar 主题 + RemoveVar 回落 + ⑤ 嵌套链（同 rt-page 节点）。
        if (page.TryGet<Container>("rt-page", out var rtPage))
        {
            if (page.TryGet<Button>("rt-theme", out var themeBtn))
                themeBtn.Clicked += () =>
                {
                    rtPage.Style.SetVar("--rt-accent", new YioColor(0.37f, 0.71f, 0.83f, 1f));
                    Say("主题·亮青");
                };
            if (page.TryGet<Button>("rt-untheme", out var unthemeBtn))
                unthemeBtn.Clicked += () =>
                {
                    rtPage.Style.RemoveVar("--rt-accent");
                    Say("已回落");
                };
            if (page.TryGet<Button>("rt-chain", out var chainBtn))
                chainBtn.Clicked += () =>
                {
                    rtPage.Style.SetVar("--rt-chain-b", new YioColor(0.75f, 0.22f, 0.17f, 1f));
                    Say("链·红");
                };
        }
        // ⑧⑨⑩ typed 重载三连（#11）：长度（border-width 变粗）/ 透明度（淡化）/
        // 字符串（fallback 消费可见化——默认紫 = var 缺席 fallback，SetVar 后红）。
        // 均 toggle：未设 → SetVar；已设 → RemoveVar 回落（CSS 声明/继承值）。
        if (page.TryGet<Container>("rt-w-block", out var wBlock)
            && page.TryGet<Button>("rt-w", out var wBtn))
        {
            bool thick = false;
            wBtn.Clicked += () =>
            {
                if (thick) { wBlock.Style.RemoveVar("--rt-border-w"); Say("边框·回落3px"); }
                else { wBlock.Style.SetVar("--rt-border-w", Length.Px(10)); Say("边框·10px"); }
                thick = !thick;
            };
        }
        if (page.TryGet<Container>("rt-fade-block", out var fadeBlock)
            && page.TryGet<Button>("rt-fade", out var fadeBtn))
        {
            bool faded = false;
            fadeBtn.Clicked += () =>
            {
                if (faded) { fadeBlock.Style.RemoveVar("--rt-alpha"); Say("透明·回落1"); }
                else { fadeBlock.Style.SetVar("--rt-alpha", 0.25f); Say("透明·0.25"); }
                faded = !faded;
            };
        }
        if (page.TryGet<Container>("rt-fb-swatch", out var fbSwatch)
            && page.TryGet<Button>("rt-fb", out var fbBtn))
        {
            bool injected = false;
            fbBtn.Clicked += () =>
            {
                if (injected) { fbSwatch.Style.RemoveVar("--rt-fb"); Say("字符串·回落fallback紫"); }
                else { fbSwatch.Style.SetVar("--rt-fb", "#c0392b"); Say("字符串·红"); }
                injected = !injected;
            };
        }
    }

    /// texture-lab 页 driver（#62 页纹理逐出验收）：
    /// 三组开关 add/remove hidden class——display:none 剪枝后该图集页失去全部可见引用，
    /// SpriteResolver 宽限期（默认 10s）满即逐出销毁；恢复显示走 GetOrLoadPage 现场重载。
    /// 读数取 driver.Host.Backend 的 SpriteResolver 统计（PagesAlive / PagesEvictedTotal）。
    /// 判据（肉眼强信号）：隐藏一组 → 约 10s 后「存活 -1、累计 +1」且其余组不动；三组错峰
    /// 隐藏则读数逐次 +1（每页独立倒计时，非批量清仓）；恢复 → 图标无缝重现、存活回升、累计不变。
    /// （主工程同款接线——双 runner 镜像规则，pitfalls「showcase 双 runner 镜像」实锚。）
    void WireTextureLabDrivers(Container page)
    {
        void Refresh()
        {
            var sprites = (_driver.Host?.Backend as UnityYioBackend)?.Sprites;
            if (sprites == null) return;
            if (page.TryGet<TextElement>("ro-alive", out var ra))
                ra.TextContent = $"存活页：{sprites.PagesAlive}";
            if (page.TryGet<TextElement>("ro-evicted", out var re))
                re.TextContent = $"累计逐出：{sprites.PagesEvictedTotal}";
        }
        void WireGroup(string btnId, string grpId, string label)
        {
            if (!page.TryGet<Button>(btnId, out var btn)
                || !page.TryGet<Container>(grpId, out var grp))
                return;
            bool hidden = false;
            btn.Clicked += () =>
            {
                hidden = !hidden;
                if (hidden) { grp.Classes.Add("hidden"); btn.TextContent = $"恢复{label}"; }
                else { grp.Classes.Remove("hidden"); btn.TextContent = $"隐藏{label}"; }
                Refresh();
            };
        }
        WireGroup("btn-grp-a", "grp-a", "A 组");
        WireGroup("btn-grp-b", "grp-b", "B 组");
        WireGroup("btn-grp-c", "grp-c", "C 组");
        if (page.TryGet<Button>("btn-refresh", out var bRefresh))
            bRefresh.Clicked += Refresh;
        Refresh();
    }

    void WireControls(Container page, string pageName)
    {
        if (pageName == "runtime-css")
        {
            WireRuntimeCssPage(page);
        }
        if (pageName == "component-lab")
        {
            WireComponentLabPage(page);
        }
        if (pageName == "shape-mask")
        {
            WireShapeMaskPage(page);
        }
        if (pageName == "texture-lab")
        {
            WireTextureLabDrivers(page);
        }
        if (pageName == "lab")
        {
            // lab #14 运行时 ZIndex：按钮把 B 片在 4（置顶）/ 0（回落 DOM 序）间切换——
            // 便签层 inline override，下帧绘制序生效（不触发 flex solve）。
            if (page.TryGet<Button>("zi-btn", out var ziBtn)
                && page.TryGet<Container>("zi-b", out var ziB))
            {
                ziBtn.Clicked += () =>
                {
                    bool raised = ziB.Style.ZIndex > 0;
                    ziB.Style.ZIndex = raised ? 0 : 4;
                    Debug.Log($"[Showcase] lab #14 B z-index -> {ziB.Style.ZIndex}");
                };
            }
        }
        if (pageName == "settings")
        {
            // Slider.ValueChanged 逐帧拖拽值 → 同步刷新旁边的数值标签。
            if (page.TryGet<Slider>("vol-master", out var vol)
                && page.TryGet<TextElement>("vol-master-val", out var volVal))
            {
                vol.ValueChanged += e => volVal.TextContent = Mathf.RoundToInt(e.NewValue).ToString();
            }
            // Toggle.CheckedChanged → 控制台输出（演示 checkbox 事件链）。
            if (page.TryGet<Toggle>("gfx-fullscreen", out var fs))
                fs.CheckedChanged += e => Debug.Log($"[Showcase] fullscreen = {e.NewValue}");
            // Dropdown.SelectionChanged（控件束 P3 typed 事件链：select 弹出列表选中）。
            if (page.TryGet<Dropdown>("gfx-res", out var res))
                res.SelectionChanged += e => Debug.Log($"[Showcase] gfx-res selected index = {e.NewIndex}");
            // NumberField.ValueChanged（控件束 P3：数值框，float 值经 min/max clamp + step 量化）。
            if (page.TryGet<NumberField>("snd-voices", out var voices))
                voices.ValueChanged += e => Debug.Log($"[Showcase] snd-voices = {e.NewValue}");
            // TextField.Submitted（控件束 P2：单行框回车提交）。
            if (page.TryGet<TextField>("key-custom", out var keyCustom))
                keyCustom.Submitted += v => Debug.Log($"[Showcase] key-custom submitted: \"{v}\"");
        }

        if (pageName == "character")
        {
            // Button.Clicked → ProgressBar.Value += 10（clamp 由 core 做），并刷新百分比标签。
            // EXP 条在 <stat-bar id="exp-bar"> 组件展开域内（投影内容归组件域）——两跳获取。
            ProgressBar exp = null;
            TextElement expVal = null;
            if (page.TryGet<ProgressBar>("stat-exp", out var expDirect))
            {
                exp = expDirect;   // 兼容未组件化的直排形态
                page.TryGet<TextElement>("stat-exp-val", out expVal);
            }
            else if (page.TryGet<CustomElement>("exp-bar", out var expBar))
            {
                expBar.TryGet<ProgressBar>("stat-exp", out exp);
                expBar.TryGet<TextElement>("stat-exp-val", out expVal);
            }
            if (page.TryGet<Button>("btn-train", out var train) && exp != null && expVal != null)
            {
                train.Clicked += () =>
                {
                    exp.Value = Mathf.Min(exp.Value + 10f, exp.Max);
                    expVal.TextContent = $"{Mathf.RoundToInt(exp.Value)}%";
                };
            }
        }

        // form 页（角色创建表单）= 控件束 P2/P3 typed 事件主力验收页：文本框全家 + Dropdown。
        // 每个变体类型各接一条事件 → Console，证明 C# 投影类的 typed 事件链全通。
        // 文本框 ValueChanged 逐字符触发（验收时输几个字符看 Console 几条 log）；Submitted 回车触发。
        if (pageName == "form")
        {
            // TextField：ValueChanged（逐字符）+ Submitted（回车提交）。
            if (page.TryGet<TextField>("char-name", out var name))
            {
                name.ValueChanged += e => Debug.Log($"[Showcase] char-name: \"{e.NewValue}\"");
                name.Submitted += v => Debug.Log($"[Showcase] char-name submitted: \"{v}\"");
            }
            // char-pass：password 掩码由 CSS -webkit-text-security:disc 声明（core 显示层
            // 变换，value 原文不变）；这里只 log 长度证明 value 未被掩码污染。
            if (page.TryGet<TextField>("char-pass", out var pass))
                pass.ValueChanged += e => Debug.Log($"[Showcase] char-pass changed (len={(e.NewValue?.Length ?? 0)})");
            // char-search：<input type="search"> 同样折叠为 TextField。
            if (page.TryGet<TextField>("char-search", out var search))
                search.ValueChanged += e => Debug.Log($"[Showcase] char-search: \"{e.NewValue}\"");
            // Dropdown.SelectionChanged（P3：select 弹出列表，typed 事件链）。
            if (page.TryGet<Dropdown>("char-class", out var cls))
                cls.SelectionChanged += e => Debug.Log($"[Showcase] char-class selected index = {e.NewIndex}");
            // 初始属性分配 slider：ValueChanged → 旁边数字标签（同 settings vol-master 模式）。
            // label 的 id 在 form.html 里（attr-str-val / attr-agi-val / attr-int-val）。
            string[] attrSliders = { "attr-str", "attr-agi", "attr-int" };
            foreach (string sid in attrSliders)
            {
                if (page.TryGet<Slider>(sid, out var attr)
                    && page.TryGet<TextElement>(sid + "-val", out var attrVal))
                {
                    Slider s = attr;
                    TextElement v = attrVal;
                    s.ValueChanged += e => v.TextContent = Mathf.RoundToInt(e.NewValue).ToString();
                }
            }
            // TextArea.ValueChanged（P2 多行变体类型对）。
            if (page.TryGet<TextArea>("char-bio", out var bio))
                bio.ValueChanged += e => Debug.Log($"[Showcase] char-bio changed (len={(e.NewValue?.Length ?? 0)})");
        }
    }

    /// ListView 虚拟化驱动：背包 / 邮件左侧列表。
    /// runtime ListView 是数据驱动的——data-fill 只供浏览器 preview 克隆（yio-preview.js），
    /// runtime 必须业务侧设 ItemCount + BindItem 才克隆 slot 渲染 item（见 Yio.Nodes ListView）。
    /// 按 index 区分图标（Image.Src 轮换）+ badge 数量 + 耐久（背包）/ 发件人 + 主题（邮件）。子节点用
    /// Query<T> 按类型取：template 蓝图克隆后 N 个 slot 子节点 id 重复，Get<T> 全局首匹配只命中
    /// 首个 slot（Nodes.cs Get gap），故不用 id。
    /// BindItem 须先于 ItemCount 设：ItemCount setter 首次会 drain_now + DrainPendingBinds 触发 BindItem。
    void WireListViews(Container page, string pageName)
    {
        if (pageName == "inventory" && page.TryGet<ListView>("inv-list", out var invList))
        {
            string[] icons = { "item-potion", "item-chest", "item-gem", "item-scroll", "item-staff", "item-wand" };
            invList.BindItem = (item, i) =>
            {
                var dur = item.Query<ProgressBar>();
                if (dur.Count > 0) dur[0].Value = (i * 7) % 100;
                var spans = item.Query<TextElement>();
                if (spans.Count > 0) spans[0].TextContent = "x" + ((i * 13) % 99 + 1);
                var img = item.Query<Image>();
                if (img.Count > 0) img[0].Src = "res/icons/" + icons[i % icons.Length] + ".png";
            };
            invList.ItemCount = 120;
        }

        if (pageName == "mail" && page.TryGet<ListView>("mail-list", out var mailList))
        {
            string[] senders = { "系统奖励", "竞技场", "公会战报", "好友留言", "商会通知", "赛季手册" };
            string[] subjects =
            {
                "每日登录奖励已发放", "本赛季排名结算完毕", "公会贡献度更新",
                "你的基地被探访了", "本周交易汇总已生成", "新赛季手册已解锁",
                "限时活动即将开启", "背包已满请及时清理"
            };
            mailList.BindItem = (item, i) =>
            {
                var spans = item.Query<TextElement>();
                if (spans.Count >= 2)
                {
                    spans[0].TextContent = senders[i % senders.Length];
                    spans[1].TextContent = subjects[i % subjects.Length];
                }
            };
            mailList.ItemCount = 100;
        }
    }
    // ── 手型光标皮肤（消费侧注册示例）──

    /// <summary>
    /// 32×32 经典 link-select 手型像素画（热点 (12,1) 在食指尖）。逐格手绘字符画 +
    /// 4 色调色板——32px 这种 1:1 小尺寸下几何体并集堆不出指缝（读作一团色块），
    /// 像素画是系统光标同款工艺。字符画 y=0 是顶行，SetPixels32 下标 0 是左下角像素
    /// ——写入按 (S-1-y) 翻行，否则整张纹理上下颠倒。
    /// </summary>
    static Texture2D BuildPixelHandCursorTexture()
    {
        const int S = 32;
        var transparent = new Color32(0, 0, 0, 0);
        var line = new Color32(28, 28, 32, 255);
        var fill = new Color32(255, 255, 255, 255);
        var shade = new Color32(216, 216, 222, 255);
        var px = new Color32[S * S];
        for (int y = 0; y < S; y++)
        {
            string row = HandArt[y];
            for (int x = 0; x < S; x++)
                px[(S - 1 - y) * S + x] = row[x] switch
                {
                    'o' => line,
                    'w' => fill,
                    's' => shade,
                    _ => transparent,
                };
        }
        var tex = new Texture2D(S, S, TextureFormat.RGBA32, false);
        tex.filterMode = FilterMode.Point;
        tex.SetPixels32(px);
        // Cursor.SetCursor 的纹理要求：RGBA32 / 可读 / 无 mip 链——Apply 第二参
        // false 保持可读（makeNoLongerReadable=true 会被 SetCursor 拒收并告警）。
        tex.Apply(false, false);
        return tex;
    }

    /// <summary>手型像素画（32 行 × 32 列，y=0 顶行）：'o' 描边 / 'w' 白填充 /
    /// 's' 阴影 / '.' 透明。食指尖 = 第 1 行 x10-13 的顶边中心（热点所在）。
    /// 填充像素的 4 邻域必须闭合（透明侧邻居必须为 'o'）。</summary>
    static readonly string[] HandArt =
    {
        "................................",
        ".........oooooo.................",
        ".........owwwwo.................",
        ".........owwwwo.................",
        ".........owwwwo.................",
        ".........owwwwo.................",
        ".........owwwwooooo.............",
        ".........owwwwowwwooooo.........",
        ".........owwwwowwwowwwooooo.....",
        ".........owwwwowwwowwwowwwo.....",
        ".........owwwwowwwowwwowwwo.....",
        ".........owwwwowwwowwwowwwo.....",
        ".........owwwwowwwowwwowwwo.....",
        "......oooowwwwowwwowwwowwwo.....",
        ".....owwwwwwwwwssssssssssso.....",
        ".....owwwwwwwwwwwwwwwwwwwwo.....",
        ".....owwwwwwwwwwwwwwwwwwwwo.....",
        ".....owwwwwwwwwwwwwwwwwwwwo.....",
        "......oooowwwwwwwwwwwwwwwwo.....",
        ".........owwwwwwwwwwwwwwwwo.....",
        ".........owwwwwwwwwwwwwwooo.....",
        ".........ooooooooooooooo........",
        "...........ooooooooooooo........",
        "...........ossssssssssso........",
        "...........owwwwwwwwwwwo........",
        "...........owwwwwwwwwwwo........",
        "...........owwwwwwwwwwwo........",
        "...........owwwwwwwwwwwo........",
        "...........ooooooooooooo........",
        "................................",
        "................................",
        "................................",
    };

    // ── component-lab 页（#20 RegisterComponent）───────────────────────────

    /// 离开 component-lab：解 OnLifecycleChanged 静态事件（防跨页泄漏——
    /// 静态事件的订阅目标是本页读数 span，页面 Dispose 后必须摘除）。
    void TeardownComponentLabPage()
    {
        LifecycleWidget.OnLifecycleChanged -= RefreshComponentLabReadouts;
        _clPartReg?.Dispose();     // 运行时 ::part 注入句柄（不跨页泄漏）
        _clPartReg = null;
    }
    IDisposable _clPartReg;

    void RefreshComponentLabReadouts()
    {
        if (_current == null) return;
        if (_current.TryGet<TextElement>("cl-conn", out var conn))
            conn.TextContent = LifecycleWidget.Connected.ToString();
        if (_current.TryGet<TextElement>("cl-disc", out var disc))
            disc.TextContent = LifecycleWidget.Disconnected.ToString();
    }

    /// component-lab 页（#20）：RegisterComponent 类绑定 + 生命周期回调摆台。
    /// 判据（肉眼强信号）：进页静态 ×2 connect（读数非零）；「再挂一个」= 组件出现 +
    /// connect 读数 +1；「销毁最后」= 组件消失 + disconnect 读数 +1；离页整页 Dispose
    /// 连带静态/动态实例全 disconnect（回页再 connect，计数累加）。
    void WireComponentLabPage(Container page)
    {
        LifecycleWidget.OnLifecycleChanged += RefreshComponentLabReadouts;
        RefreshComponentLabReadouts();   // 进页 connect 已发生（instantiate 早于 wire）
        TextElement status = page.TryGet<TextElement>("cl-status", out var st) ? st : null;
        void Say(string s)
        {
            if (status != null) status.TextContent = s;
            Debug.Log($"[Showcase] comp: {s}");
        }

        if (page.TryGet<Container>("cl-stage", out var stage))
        {
            // 重复注册 fail-loud（#20）：同 tag 二次注册必须抛 UIContractException。
            if (page.TryGet<Button>("cl-dup", out var dupBtn))
                dupBtn.Clicked += () =>
                {
                    try
                    {
                        _driver.Context.RegisterComponent("lifecycle-widget",
                            (c, id) => new LifecycleWidget(c, id));
                        Say("重复注册未拒?");
                    }
                    catch (UIContractException)
                    {
                        Say("重复注册→拒绝 ✓");
                    }
                };
            // 运行时 ::part 注入（#57 × #11）：StyleSheet.Add 的 ::part 通道——注入后
            // 舞台区两组件的 title 整组变红（同 specificity 后 Add 赢，覆盖 .lw-hot 金）。
            if (page.TryGet<Button>("cl-rt", out var rtBtn))
                rtBtn.Clicked += () =>
                {
                    _clPartReg?.Dispose();
                    // specificity 对齐烘焙规则（.lw-hot::part(title) = (0,2,1)）：注入同选择器，
                    // 同优先后 Add 赢（#11 cascade 语义）→ 金组件翻红；裸 ::part (0,1,1) 会输。
                    _clPartReg = _driver.Context.StyleSheet.Add(
                        ".lw-hot::part(title) { color: #e74c3c; }");
                    Say("已注入 ::part 红");
                };
            // 列表换绑淘汰（#20 pump 路径）：ItemCount 减员 → core 杀克隆 → 下帧
            // PumpRemovedNodes 对已物化 wrapper fire OnDisconnected（disc 跳增）。
            if (page.TryGet<ListView>("cl-list", out var lv))
            {
                // 模板句柄必须在 ItemCount（enter_data_driven 收编并清掉 <template> 子）之前
                // 全部取好——enter 之后再 GetTemplate 会「not found in scope」抛异常
                //（api-infra 同款顺序：全部 GetTemplate → 模板设定 → BindItem → ItemCount）。
                UITemplate rowWidget = lv.GetTemplate("cl-row");
                UITemplate rowPlain = lv.GetTemplate("cl-row-plain");
                lv.ItemTemplate = rowWidget;
                // BindItem 触达子树（Query 物化 widget wrapper → OnConnected）；
                // 不触达则克隆永不物化、死亡无 wrapper 可通知。
                lv.BindItem = (item, i) =>
                {
                    // 触达即物化：Query 构造 widget wrapper（→ OnConnected）；
                    // 不触达则克隆永不物化、死亡无 wrapper 可通知。
                    var _ = item.Query<CustomElement>();
                };
                lv.ItemCount = 8;
                if (page.TryGet<Button>("cl-swap", out var swapBtn))
                {
                    ListView captured = lv;
                    swapBtn.Clicked += () =>
                    {
                        // Rust 侧死亡触发器 = 整列表销毁（蓝图/全部克隆一次释放——park 语义下
                        // 换蓝图/减员都不 free 节点，那是池化不是死亡）。disc 大跳增即泵实证。
                        captured.Dispose();
                        Say("列表已销毁（disc 跳增；重新进页复原）");
                    };
                }
            }
            if (page.TryGet<Button>("cl-pop", out var popBtn))
            {
                // 销毁动态挂的最后一个（按挂载记录，不按 Query 序——DFS 序与视觉序不必然一致，
                // 按文档尾取会销毁错对象：金色静态组件可能恰在末位）。
                System.Collections.Generic.List<Container> added = new System.Collections.Generic.List<Container>();
                if (page.TryGet<Button>("cl-add", out var addBtn))
                    addBtn.Clicked += () =>
                    {
                        Container holder = _driver.Instantiate("showcase", "widget-holder");
                        if (holder == null) { Debug.Log("[Showcase] comp: widget-holder instantiate FAIL"); return; }
                        holder.RemoveFromParent();
                        stage.AddChild(holder);
                        added.Add(holder);
                        Debug.Log($"[Showcase] comp: +1 (conn={LifecycleWidget.Connected} disc={LifecycleWidget.Disconnected})");
                    };
                popBtn.Clicked += () =>
                {
                    if (added.Count == 0) { Say("没有动态组件可销毁（先点「再挂一个」）"); return; }
                    Container last = added[added.Count - 1];
                    added.RemoveAt(added.Count - 1);
                    last.Dispose();
                    Debug.Log($"[Showcase] comp: -1 (conn={LifecycleWidget.Connected} disc={LifecycleWidget.Disconnected})");
                };
            }
        }
    }
}

/// <summary>
/// #20 RegisterComponent 摆台用的 typed 组件子类：OnConnected/OnDisconnected 静态计数 +
/// 变更事件（页面读数刷新）。构造链 protected internal 基类构造；工厂委托在
/// ShowcaseRunner.Start 注册（setup 期）。
/// </summary>
public class LifecycleWidget : CustomElement
{
    public static int Connected;
    public static int Disconnected;
    public static event Action OnLifecycleChanged;

    public LifecycleWidget(UIContext ctx, ulong id) : base(ctx, id) { }

    protected override void OnConnected()
    {
        Connected++;
        OnLifecycleChanged?.Invoke();
    }

    protected override void OnDisconnected()
    {
        Disconnected++;
        OnLifecycleChanged?.Invoke();
    }
}
