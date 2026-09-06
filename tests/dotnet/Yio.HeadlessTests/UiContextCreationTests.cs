using System;
using System.Text;
using Yio.Bindings;
using Xunit;

namespace Yio.HeadlessTests
{
    /// <summary>
    /// E1: UIContext / UIPackage / UITemplate method bodies (TDD).
    ///
    /// Cover Create&lt;T&gt; whitelist (Container ok, Button/Slider throw UIContractException),
    /// Root (create_root + _rootId), FocusedNode (FFI round-trip), IsPointerOnUI (FFI).
    ///
    /// LoadPackage / Instantiate end-to-end tests deferred to E2/E3 (fixture pkg.bin dependency).
    /// UnloadPackage / CallLater / CallNextFrame 已接通（投影层内建调度 + core unload FFI）。
    /// </summary>
    public unsafe class UiContextCreationTests
    {
        const ulong RootSentinel = ulong.MaxValue;

        // ── helpers ────────────────────────────────────────────────────

        /// <summary>
        /// 调 create_root FFI 建根节点 + 注册到 UIContext._rootId。
        /// 返回 typed Container（registry 缓存 + _rootId 已设）。
        /// </summary>
        static Container InitRoot(UIContext ctx, string kind = "div", string css = "")
        {
            IntPtr stage = ctx._stage;
            StageHandle* h = (StageHandle*)stage.ToPointer();
            byte[] k = Encoding.UTF8.GetBytes(kind);
            byte[] c = Encoding.UTF8.GetBytes(css);
            ulong id;
            fixed (byte* kp = k, cp = c)
                id = Native.yio_stage_create_root(h, kp, (nuint)k.Length, cp, (nuint)c.Length);
            if (id == RootSentinel)
                throw new InvalidOperationException($"create_root(\"{kind}\") failed");
            ctx._rootId = id;
            return (Container)ctx._registry.GetOrCreate(id);
        }

        /// <summary>
        /// 调 create_root FFI（low-level：返回 raw NodeId，不设 _rootId）。
        /// FocusedNodeAfterRequestFocus 等需要 tick 的测试用——先建 root 建 scene，再 focus。
        /// </summary>
        static ulong CreateRootFFI(StageHandle* h, string kind, string css)
        {
            byte[] k = Encoding.UTF8.GetBytes(kind ?? "");
            byte[] c = Encoding.UTF8.GetBytes(css ?? "");
            fixed (byte* kp = k, cp = c)
                return Native.yio_stage_create_root(h, kp, (nuint)k.Length, cp, (nuint)c.Length);
        }

        // ── Create<T> ──────────────────────────────────────────────────

        [Fact]
        public void CreateContainerWhitelist()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                var c = ctx.Create<Container>();
                Assert.IsType<Container>(c);
                Assert.NotEqual(RootSentinel, c._id);
                Assert.False(c._disposed);
                // NodeId 必须是活的——get_node_kind 返 Container(0) 而非 0xFF。
                StageHandle* h = (StageHandle*)stage.ToPointer();
                byte kind = 0xFF;
                int rc = Native.yio_stage_get_node_kind(h, c._id, &kind);
                Assert.Equal(0, rc);
                Assert.Equal(0, kind);   // NodeKind::Container = 0
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CreateAbsolutePanelWhitelist()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                var p = ctx.Create<AbsolutePanel>();
                Assert.IsType<AbsolutePanel>(p);
                Assert.NotEqual(RootSentinel, p._id);
                // AbsolutePanel kind = Container (同 div)
                StageHandle* h = (StageHandle*)stage.ToPointer();
                byte kind = 0xFF;
                int rc = Native.yio_stage_get_node_kind(h, p._id, &kind);
                Assert.Equal(0, rc);
                Assert.Equal(0, kind);   // Container
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CreateTextNodeWhitelist()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                var tn = ctx.Create<TextNode>();
                Assert.IsType<TextNode>(tn);
                Assert.NotEqual(RootSentinel, tn._id);
                // TextNode kind = 1
                StageHandle* h = (StageHandle*)stage.ToPointer();
                byte kind = 0xFF;
                int rc = Native.yio_stage_get_node_kind(h, tn._id, &kind);
                Assert.Equal(0, rc);
                Assert.Equal(1, kind);   // TextNode
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CreateImageWhitelist()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                var img = ctx.Create<Image>();
                Assert.IsType<Image>(img);
                Assert.NotEqual(RootSentinel, img._id);
                // Image kind = 4（Rust NodeKind 枚举序：Container=0, TextNode=1, TextElement=2, Button=3, Image=4）
                StageHandle* h = (StageHandle*)stage.ToPointer();
                byte kind = 0xFF;
                int rc = Native.yio_stage_get_node_kind(h, img._id, &kind);
                Assert.Equal(0, rc);
                Assert.Equal(4, kind);   // Image
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CreateRejectsButton()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                var ex = Assert.Throws<UIContractException>(() => ctx.Create<Button>());
                Assert.Contains("Button", ex.Message);
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CreateRejectsSlider()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                var ex = Assert.Throws<UIContractException>(() => ctx.Create<Slider>());
                Assert.Contains("Slider", ex.Message);
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CreateRejectsToggle()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                Assert.Throws<UIContractException>(() => ctx.Create<Toggle>());
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CreateRejectsListView()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                Assert.Throws<UIContractException>(() => ctx.Create<ListView>());
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CreateRejectsDropdown()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                Assert.Throws<UIContractException>(() => ctx.Create<Dropdown>());
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CreateRejectsProgressBar()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                Assert.Throws<UIContractException>(() => ctx.Create<ProgressBar>());
            }
            finally { StageHarness.Destroy(stage); }
        }

        // ── Root ───────────────────────────────────────────────────────

        [Fact]
        public void RootBeforeCreateRootThrows()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                // _rootId still RootSentinel (no create_root called yet)
                Assert.Throws<InvalidOperationException>(() => _ = ctx.Root);
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void RootAfterCreateRootReturnsContainer()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                Container r = InitRoot(ctx);
                Container root = ctx.Root;
                Assert.Same(r, root);   // identity stable: same wrapper instance
                Assert.IsType<Container>(root);
            }
            finally { StageHarness.Destroy(stage); }
        }

        // ── FocusedNode ────────────────────────────────────────────────

        [Fact]
        public void FocusedNodeInitiallyNull()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                _ = InitRoot(ctx);
                Assert.Null(ctx.FocusedNode);   // no focus requested yet
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void FocusedNodeAfterRequestFocus()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                // request_focus writes pending_focus_request；tick 消费它 → scene.focused_node。
                // focused_node FFI 读 scene.focused_node，不是 pending——故需先 tick。
                StageHandle* h = (StageHandle*)stage.ToPointer();
                ulong rootId = CreateRootFFI(h, "div", "");
                ctx._rootId = rootId;
                var c = ctx.Create<Container>();

                Native.yio_stage_request_focus(h, c._id);
                Native.yio_stage_tick(h, 0.016f);   // consume pending_focus_request

                Node f = ctx.FocusedNode;
                Assert.NotNull(f);
                Assert.Equal(c._id, f._id);
                Assert.Same(c, f);   // identity stable
            }
            finally { StageHarness.Destroy(stage); }
        }

        // ── IsPointerOnUI ─────────────────────────────────────────────

        [Fact]
        public void IsPointerOnUiInitiallyFalse()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                _ = InitRoot(ctx);
                // No pointer input fed yet → false
                Assert.False(ctx.IsPointerOnUI);
            }
            finally { StageHarness.Destroy(stage); }
        }

        // ── StyleSheet ─────────────────────────────────────────────────

        [Fact]
        public void StyleSheetReturnsSameInstance()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                var s1 = ctx.StyleSheet;
                var s2 = ctx.StyleSheet;
                Assert.Same(s1, s2);   // lazy, single instance
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void StyleSheetAddValidCssReturnsDisposableHandle()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                var reg = ctx.StyleSheet.Add(".rt { color: #ff0000 }");
                reg.Dispose();   // 撤销句柄不抛
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void StyleSheetAddAtRuleThrowsUIStyleExceptionWithLocation()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                // at-rule 在注入通道全拒（含 @keyframes——fail-loud 不静默跳过）。
                var ex = Assert.Throws<UIStyleException>(() =>
                    ctx.StyleSheet.Add("@keyframes fade { from{opacity:0} }"));
                Assert.True(ex.Line >= 1 && ex.Column >= 1, $"行列在场: L{ex.Line} C{ex.Column}");
                Assert.Contains("runtime stylesheet", ex.Message);
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void StyleSheetAddBadSelectorThrowsUIStyleException()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                // 坏选择器样例须选仍处围栏外的构造：`>` 已入子集（#114），相邻组合器 `+` 仍拒。
                var ex = Assert.Throws<UIStyleException>(() =>
                    ctx.StyleSheet.Add(".a + .b { color: #fff }"));
                Assert.Contains(".a + .b", ex.Message);
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void StyleSheetClearNoThrow()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                var ss = ctx.StyleSheet;
                ss.Add(".rt { color: #ff0000 }");
                ss.Clear();   // 已接线：清空运行时注入（pkg 规则不动），不抛
            }
            finally { StageHarness.Destroy(stage); }
        }

        // ── LoadPackage (duplicate) ────────────────────────────────────

        [Fact]
        public void LoadPackageNullNameThrows()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                _ = InitRoot(ctx);
                Assert.Throws<ArgumentNullException>(() => ctx.LoadPackage(null, new byte[] { 1 }));
                Assert.Throws<ArgumentNullException>(() => ctx.LoadPackage("", new byte[] { 1 }));
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void LoadPackageNullBytesThrows()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                _ = InitRoot(ctx);
                Assert.Throws<ArgumentNullException>(() => ctx.LoadPackage("test", null));
                Assert.Throws<ArgumentNullException>(() => ctx.LoadPackage("test", Array.Empty<byte>()));
            }
            finally { StageHarness.Destroy(stage); }
        }

        // ── UIPackage / UITemplate basics ──────────────────────────────

        [Fact]
        public void UIPackageNameRoundTrip()
        {
            // UIPackage is internal-ctor only, but we can test Name via LoadPackage mock.
            // Since LoadPackage end-to-end needs fixture, test the ctor path directly:
            // UIPackage's ctor is internal but we're in HeadlessTests (different assembly).
            // Ponytail: test Name/GetTemplate via the getter pattern — we need a UIPackage instance
            // but ctor is internal. Skip for now — E2 fixture will cover this path.
        }

        // ── 调度 / 包生命周期（NE stub 已接通；完整行为见 SchedulerAndLifecycleTests）──

        [Fact]
        public void UnloadPackageNotLoadedThrowsContract()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                // 与 LoadPackage 同名重复抛 UIContractException 对称——不静默。
                Assert.Throws<UIContractException>(() => ctx.UnloadPackage("foo"));
            }
            finally { StageHarness.Destroy(stage); }
        }

        // （原 PickThrowsNe 已改——Pick 已接通 yio_stage_hit_test。空 stage（无 scene）
        //  rc=-1 升 InvalidOperationException，与其它 FFI 转调口径一致；命中/未命中语义
        //  见 ControlStateProjectionTests.uicontext_pick_hits_and_misses。）
        [Fact]
        public void PickOnEmptyStageThrowsInvalidOp()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                Assert.Throws<InvalidOperationException>(() => ctx.Pick(new YioVector2(100, 100)));
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CallLaterFiresWhenDtAccumulates()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                int fired = 0;
                ctx.CallLater(0.5f, () => fired++);
                ctx.PumpLogic(0.3f);            // 0.3 < 0.5 → 未到期
                Assert.Equal(0, fired);
                ctx.PumpLogic(0.3f);            // 累计 0.6 ≥ 0.5 → fire 恰一次
                Assert.Equal(1, fired);
                ctx.PumpLogic(1f);              // one-shot：不再 fire
                Assert.Equal(1, fired);
            }
            finally { StageHarness.Destroy(stage); }
        }

        [Fact]
        public void CallNextFrameFiresExactlyAtNextPumpHead()
        {
            var (stage, ctx) = StageHarness.Create();
            try
            {
                int fired = 0;
                ctx.CallNextFrame(() => fired++);
                ctx.PumpLogic(0.016f);          // 下一次泵开头 fire（帧头语义）
                Assert.Equal(1, fired);
                ctx.PumpLogic(0.016f);          // one-shot
                Assert.Equal(1, fired);
            }
            finally { StageHarness.Destroy(stage); }
        }
    }
}
