using System;
using System.Buffers;   // ArrayPool<byte> for _frameBuf
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using LoomGUI.Bindings;
using UnityEngine;
using UnityEngine.U2D;   // SpriteAtlas（v1.4-a T8 path→Sprite 查询）

namespace LoomGUI
{
    /// 与 Rust tween::TweenProp (u8) 对齐。
    public enum TweenProp : byte { Opacity = 0, Translate = 1, Scale = 2, Rotation = 3, BgColor = 4, TextColor = 5 }
    /// 与 Rust tween::Ease (u8) 对齐。
    public enum Ease : byte { Linear = 0, QuadIn = 1, QuadOut = 2, QuadInOut = 3, CubicIn = 4, CubicOut = 5, CubicInOut = 6, BackIn = 7, BackOut = 8, BackInOut = 9 }

    /// <summary>
    /// 集成入口：把 Rust Stage（tick→borrow_frame→blob）接到 Unity MirrorPool 渲染。
    /// 挂场景即跑：Awake 建 stage+pool+SpriteResolver+配置根/相机；LateUpdate 每帧
    /// tick→borrow→Marshal.Copy→FrameBlob→MirrorPool.Sync。
    ///
    /// v1.4-a T8：包加载重构——Awake 不再自动 load 单包建 scene。业务 driver 调
    /// CreateRoot 建 scene → LoadPackage(name, bytes) 进资源池 → Instantiate(pkg, comp) 建内容。
    /// 图片走 path→Sprite（Sprite Atlas），不再 LoadAtlas/_texMap/tex_id。
    ///
    /// 设计坐标系：origin 左上、y-down，单位 design px（_designSize）。根 transform 一次做
    /// MatchWidthOrHeight 缩放 + y-flip（localScale=(sf,-sf,sf)）+ 平移到屏幕左上原点。
    /// UI 相机正交、cullingMask=LoomUI(layer 6)、orthoSize=Screen.height/2，独立于根（不被
    /// 根的负 scale 影响）。shader Cull Off 吸收翻转的 winding。
    /// </summary>
    [ExecuteAlways]
    public sealed unsafe class LoomStage : MonoBehaviour
    {
        [SerializeField] Vector2 _designSize = new(1080, 1920);
        [SerializeField] Camera _uiCamera;
        // Unity 动态字体：与 Rust measure 的同一份 DejaVuSans.ttf（Assets/LoomGUI/Fonts/）。
        // Inspector 指定为主路径；EditMode 测试 / 未配场景用 AssetDatabase 兜底（见 EnsureFont）。
        [SerializeField] Font _font;

        // Stage 读取的 ttf 文件名（StreamingAssets 下，喂 Rust measure）。
        // 默认 "DejaVuSans.ttf"；CJK sample 配 "wqy-microhei.ttc"。
        // 须与 _font（Unity 光栅用）是同一份 ttf（跨平台一致性）。
        [SerializeField] string _fontFile = "DejaVuSans.ttf";

        // v1.4-a T8：Sprite Atlas 接入。开发者建 SpriteAtlas asset（把 res/ 下 Sprite 划进去），
        // Inspector 拖入此列表。LoomStage Awake 时注册进 SpriteResolver，MirrorPool 按 path 查 Sprite。
        // 多图集：path 路由到对应 atlas 是 Unity 内部事（核心不感知）。
        [SerializeField] List<SpriteAtlas> _spriteAtlases = new();

        // on-screen FPS 读数。stress500 已砍（v1.4-a 改包加载模型，原内联 html fixture 无加载路径；
        // stress 测试改用内存 pkg + instantiate）。本开关保留供 driver 手动开 FPS 显示。
        [SerializeField] bool _showFps;

        // safe-area 根 letterbox（默认 on）。on 时根 shrink-to-fit 到 Screen.safeArea，
        // 内容自动避刘海；off 时全屏。无刘海屏 safeArea==全屏 → 零回归。
        [SerializeField] bool _safeArea = true;

        // csbindgen 生成的 Native 用类型化指针 StageHandle*（非 IntPtr）；
        // 借出长度参数是 nuint*（非 ulong*）。故本类标 unsafe 并持 StageHandle*。
        StageHandle* _stage;
        MaterialManager _mm;
        MirrorPool _pool;
        NativeHostManager _nhm;
        // v1.4-a T8：path → Sprite 查询（替代 _texMap）。MirrorPool 按 path_idx→path→GetSprite 查。
        SpriteResolver _sprites;
        // ArrayPool 租用（非 new）。Rent 返回 ≥len，只 copy/解析 len 字节。
        // OnDestroy 归还防泄漏。冷帧零 GC（ReadMesh per-node alloc 留观察，撞墙再上 List 复用）。
        byte[] _frameBuf;

        // 输入采集 + 事件派发（Inspector 指定；为 null 时跳过输入/事件路径）。
        // _inputCollector 通常与本 MonoBehaviour 同 GO（Awake 时 GetComponent 兜底）。
        [SerializeField] LoomInputCollector _inputCollector;
        // LoomEventHandler 非 UnityEngine.Object（纯 C# class）——不能 SerializeField 持有
        // （Unity 不序列化非 Object 引用字段为资产链接）。改为 Awake new + 外部 AddListener 注册。
        readonly LoomEventHandler _eventHandler = new();

        // 上一帧 Screen 尺寸，检测 resize 重配根/相机。
        int _lastScreenW = -1, _lastScreenH = -1;

        const int LoomUILayer = 6;

        /// 游戏侧通过此属性注册 listener（AddListener/RemoveListener），例如
        /// stage.EventHandler.AddListener(nodeId, EventType.Click, OnBtnClick)。
        public LoomEventHandler EventHandler => _eventHandler;

        /// 暴露给 LoomInputCollector.CollectWheel + demo 等内部消费者。
        internal System.IntPtr StagePtr => (System.IntPtr)_stage;
        internal Vector2 DesignSize => _designSize;
        internal bool UseSafeArea => _safeArea;

        /// UI 挡住时游戏不响应点击。= 任一活跃槽（鼠标 + 触摸）命中非根节点。
        /// 游戏侧每帧/点击时查此 bool 决定是否消费输入（true → 游戏不响应）。
        public bool IsPointerOnUI()
        {
            if (_stage == null) return false;
            return Native.loomgui_stage_is_pointer_on_ui(_stage);
        }

        /// 按 CSS id 属性查节点（替代硬编码 build 序 id——auto Text 子会偏移序，不可靠）。
        /// 返 node_id；无匹配 / stage 未建 → uint.MaxValue（0xFFFF_FFFF）。
        public uint FindNodeById(string id)
        {
            if (_stage == null) return uint.MaxValue;
            byte[] bytes = Encoding.UTF8.GetBytes(id);
            fixed (byte* p = bytes)
                return Native.loomgui_stage_find_node_by_id(_stage, p, (nuint)bytes.Length);
        }

        /// 业务设节点 disabled（伪类源 + active/click 抑制）。NodeId 越界 native 侧静默跳过。
        public void SetNodeDisabled(uint nodeId, bool disabled)
        {
            if (_stage == null) return;
            Native.loomgui_stage_set_node_disabled(_stage, nodeId, disabled);
        }

        /// 编程滚动到指定位置。非 scroll 容器 / 越界 node → no-op（不 panic）。
        /// animated: true → cubic-out 缓动；false → 瞬移。
        public void SetScrollPos(uint node, float x, float y, bool animated = true)
        {
            if (_stage == null) return;
            Native.loomgui_stage_set_scroll_pos(_stage, node, x, y, animated ? (byte)1 : (byte)0);
        }

        /// 绑定外部 GO 到 UI 节点（NativeHost-lite spec）。
        /// 每帧 Sync 时自动同步 TRS + visible + sortingOrder。
        public void BindNativeHost(uint nodeId, GameObject go) => _nhm.Bind(nodeId, go);

        /// 按 CSS id 查 nodeId 后绑定外部 GO。
        public void BindNativeHost(string id, GameObject go)
        {
            uint nodeId = FindNodeById(id);
            if (nodeId == uint.MaxValue) { Debug.LogError($"[LoomGUI] NativeHost bind: id '{id}' not found"); return; }
            _nhm.Bind(nodeId, go);
        }

        public void UnbindNativeHost(uint nodeId) => _nhm.Unbind(nodeId);

        /// dump 当前 scene 为 JSON（Rust 拥有，下 tick 失效）。
        public string DumpScene()
        {
            if (_stage == null) return "[]";
            unsafe
            {
                nuint len;
                byte* p = Native.loomgui_stage_dump_scene(_stage, &len);
                if (p == null) return "[]";
                return Encoding.UTF8.GetString(p, (int)len);
            }
        }

        /// 注册 tween。start/end 取前 value_size 个分量（prop 决定）。
        /// 例：fade-in → Tween(id, TweenProp.Opacity, new[]{0f,0,0,0}, new[]{1f,0,0,0}, 0.3f, Ease.Linear, 0f, tag)。
        public void Tween(uint nodeId, TweenProp prop, float[] start, float[] end, float duration, Ease ease, float delay, uint tag)
        {
            if (_stage == null) return;
            unsafe
            {
                fixed (float* sp = start, ep = end)
                    Native.loomgui_stage_tween(_stage, nodeId, (uint)prop, sp, ep, duration, (uint)ease, delay, tag);
            }
        }

        public void KillTween(uint nodeId, TweenProp prop)
        {
            if (_stage == null) return;
            Native.loomgui_stage_kill_tween(_stage, nodeId, (uint)prop);
        }

        public void ClearAnim(uint nodeId)
        {
            if (_stage == null) return;
            Native.loomgui_stage_clear_anim(_stage, nodeId);
        }

        public void ClearAnimProp(uint nodeId, TweenProp prop)
        {
            if (_stage == null) return;
            Native.loomgui_stage_clear_anim_prop(_stage, nodeId, (uint)prop);
        }

        // ===== v1.4-a T8 包加载 API（§4 load_package/instantiate）：转调 FFI（T7 csbindgen 生成）。
        // 包 = 资源池里的组件模板库；load_package 只进资源池不建 scene；instantiate 克隆子树进 scene。
        // 调用流程（业务 driver）：CreateRoot 建 scene → LoadPackage(name,bytes) 进资源池 →
        // Instantiate(pkg,comp) 建内容 → AppendChild 挂 layer。

        /// 加载包进 Stage 资源池（不建 scene）。多包共存（多次调，name 区分）。
        /// name = 包名（UTF-8，对齐 T4 Stage::load_package(name, bytes)）；bytes = .pkg.bin 二进制。
        /// 返 0=ok，-1=err（stage 未建 / native 解析失败）。包是 Rust-internal，C# 只透传 bytes（不解析）。
        public int LoadPackage(string name, byte[] bytes)
        {
            if (_stage == null) return -1;
            byte[] nb = Encoding.UTF8.GetBytes(name ?? "");
            fixed (byte* np = nb, bp = bytes)
            {
                int r = Native.loomgui_stage_load_package(
                    _stage, np, (nuint)nb.Length, bp, (nuint)(bytes?.Length ?? 0));
                return r;
            }
        }

        /// 从包克隆组件子树进当前 scene，返组件根 NodeId（孤立，调用方 AppendChild 挂 layer）。
        /// pkg = 包名；comp = 组件名（HTML 文件名去 .html）。返 0xFFFF_FFFF = 失败（无 scene / 包/组件不存在）。
        public uint Instantiate(string pkg, string comp)
        {
            if (_stage == null) return uint.MaxValue;
            byte[] pb = Encoding.UTF8.GetBytes(pkg ?? "");
            byte[] cb = Encoding.UTF8.GetBytes(comp ?? "");
            fixed (byte* pp = pb, cp = cb)
                return Native.loomgui_stage_instantiate(
                    _stage, pp, (nuint)pb.Length, cp, (nuint)cb.Length);
        }

        // ===== 动态树 API 封装（§7.2）：转调 FFI（T7 csbindgen 生成 Native.loomgui_stage_*）。
        // kind/css/text/src = UTF-8 字节（fixed 钉住 + 指针+len，同 FindNodeById 风格）。
        // create_root/create_node 返 uint NodeId（0xFFFF_FFFF = 失败）；其余返 int（0=ok，-1=err）。
        // 调用方：用返回的 NodeId 句柄，勿硬编码 0（slotmap idx 从 1 起 → 首节点 NodeId 非 0）。
        // 前置：须先 CreateRoot 建 scene（create_node 等需 self.scene Some）。

        /// 建根节点并设为 roots[0]。kind ∈ {div/l-container/button/img/span}；css = "w:100px;..."。
        /// 返 NodeId；0xFFFF_FFFF = 失败（无 scene / 未知 kind）。
        public uint CreateRoot(string kind, string css)
        {
            if (_stage == null) return uint.MaxValue;
            byte[] k = Encoding.UTF8.GetBytes(kind ?? "");
            byte[] c = Encoding.UTF8.GetBytes(css ?? "");
            fixed (byte* kp = k, cp = c)
                return Native.loomgui_stage_create_root(_stage, kp, (nuint)k.Length, cp, (nuint)c.Length);
        }

        /// 建游离节点（不挂父）。需配合 AppendChild/InsertBefore 挂到树。
        /// 返 NodeId；0xFFFF_FFFF = 失败。
        public uint CreateNode(string kind, string css)
        {
            if (_stage == null) return uint.MaxValue;
            byte[] k = Encoding.UTF8.GetBytes(kind ?? "");
            byte[] c = Encoding.UTF8.GetBytes(css ?? "");
            fixed (byte* kp = k, cp = c)
                return Native.loomgui_stage_create_node(_stage, kp, (nuint)k.Length, cp, (nuint)c.Length);
        }

        /// 挂子到 parent 末尾。child 必须当前无父。返 0=ok，-1=err。
        public int AppendChild(uint parent, uint child)
        {
            if (_stage == null) return -1;
            return Native.loomgui_stage_append_child(_stage, parent, child);
        }

        /// 在 parent.children 中 refId 之前插 child。refId=0xFFFF_FFFF → 末尾追加。
        /// 返 0=ok，-1=err。
        public int InsertBefore(uint parent, uint child, uint refId)
        {
            if (_stage == null) return -1;
            return Native.loomgui_stage_insert_before(_stage, parent, child, refId);
        }

        /// 摘子（不删节点）：从 parent.children 移除，节点仍 live 可重挂。返 0=ok，-1=err。
        public int RemoveChild(uint parent, uint child)
        {
            if (_stage == null) return -1;
            return Native.loomgui_stage_remove_child(_stage, parent, child);
        }

        /// 删节点（递归删子 + 联动清 anim/scroll/tween + slotmap remove）。
        /// 旧 NodeId 此后失效（gen++）。返 0（恒成功，no-op 语义）。
        public int RemoveNode(uint node)
        {
            if (_stage == null) return 0;
            return Native.loomgui_stage_remove_node(_stage, node);
        }

        /// 改 Text 节点 content + 标 dirty_text。非 Text 节点 → -1。返 0=ok，-1=err。
        public int SetText(uint node, string text)
        {
            if (_stage == null) return -1;
            byte[] t = Encoding.UTF8.GetBytes(text ?? "");
            fixed (byte* tp = t)
                return Native.loomgui_stage_set_text(_stage, node, tp, (nuint)t.Length);
        }

        /// 改 Image 节点 src + 标 dirty_mesh。非 Image 节点 → -1。返 0=ok，-1=err。
        public int SetSrc(uint node, string src)
        {
            if (_stage == null) return -1;
            byte[] s = Encoding.UTF8.GetBytes(src ?? "");
            fixed (byte* sp = s)
                return Native.loomgui_stage_set_src(_stage, node, sp, (nuint)s.Length);
        }

        /// 改 base_style（apply_css）+ 标 dirty_mesh。下帧 rematch 从 base 重算 style。
        /// 返 0=ok，-1=err。
        public int SetStyle(uint node, string css)
        {
            if (_stage == null) return -1;
            byte[] c = Encoding.UTF8.GetBytes(css ?? "");
            fixed (byte* cp = c)
                return Native.loomgui_stage_set_style(_stage, node, cp, (nuint)c.Length);
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

            // _stage 创建后即 SetHandle（handler.node_parent FFI 需 StageHandle*）。
            _eventHandler.SetHandle((System.IntPtr)_stage);

            // v1.4-a T8：不再自动 load 单包建 scene。业务 driver 调 CreateRoot 建 scene →
            // LoadPackage(name,bytes) 进资源池 → Instantiate(pkg,comp) 建内容。Awake 只建基础设施。
            var shader = Shader.Find("LoomGUI/Unlit");
            if (shader == null)
            {
                Debug.LogError("[LoomStage] 找不到 Shader \"LoomGUI/Unlit\"——URP shader 是否在包内？");
                FreeStage();
                return;
            }
            _mm = new MaterialManager(shader);
            _pool = new MirrorPool();
            _nhm = new NativeHostManager(); _nhm.Init(transform);

            // v1.4-a T8：path→Sprite 查询（替代 LoadAtlas/_texMap）。注册 Inspector 配的 SpriteAtlas。
            // 开发者建 SpriteAtlas asset（res/ 下 Sprite 划进去），Inspector 拖入 _spriteAtlases。
            // MirrorPool 按 blob path_idx→path→GetSprite 查 Sprite（懒查 + 缓存）。
            _sprites = new SpriteResolver();
            if (_spriteAtlases != null) _sprites.RegisterAtlases(_spriteAtlases);

            EnsureFont();
            // Font.textureRebuilt 是静态事件：atlas 异步 rebuild 时 glyph UV 变。
            // 注册 TextRasterizer.OnRebuilt（自增 FontVersion）→ MirrorPool.Sync 下帧检测到版本
            // 变 → 强制 text 节点重 RequestCharactersInTexture + 重取 UV（照 fgui DynamicFont.cs:356-375）。
            // 全局静态事件：必须 OnDestroy 解绑，否则泄漏跨场景/实例。
            Font.textureRebuilt += TextRasterizer.OnRebuilt;

            gameObject.layer = LoomUILayer;
            EnsureCamera();
            ConfigureTransforms();

            // _inputCollector 未在 Inspector 指定时，兜底取同 GO 上的组件
            // （常见用法：LoomInputCollector 挂在 LoomStage 同 GameObject）。
            if (_inputCollector == null) _inputCollector = GetComponent<LoomInputCollector>();
        }

        /// <summary>
        /// 取 Unity 动态 Font。Inspector 指定优先；未配则 EditMode 用 AssetDatabase 兜底加载
        /// Assets/LoomGUI/Fonts/DejaVuSans.ttf（PlayMode/build 必须由用户在 Inspector 指定——
        /// AssetDatabase 仅 editor）。Font 须与 Rust measure 同一份 ttf（跨平台一致性根）。
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

        // v1.4-a T8：砍 LoadHtml / LoadPackage()（private）/ LoadPackageFile / LoadAtlas /
        // BuildStress500Fixture。包加载改走 public LoadPackage(name, bytes) + Instantiate（见上）。
        // 图片走 path→Sprite（SpriteResolver），不再 LoadAtlas/_texMap/tex_id。
        // stress500 fixture 随 inline 路径一起砍——stress 测试改用内存 pkg + instantiate（T11 driver）。

        /// <summary>
        /// on-screen FPS 读数（_showFps 为真时显示）。
        /// 1/Time.smoothDeltaTime 平滑帧率 + MirrorPool 当前节点数。最小实现（不做 profiler）。
        /// 用户在 PlayMode 肉眼判卡顿。
        /// </summary>
        void OnGUI()
        {
            if (!_showFps) return;
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
        /// design→screen 根变换（sf + rootPos）。_safeArea=true 时 shrink-to-fit 到 Screen.safeArea
        /// 并把设计 span 居中进 safe 区（safe 区外 letterbox，避刘海）；false 时全屏。
        /// 相机 orthoSize 不变（仍=sh/2 覆盖全屏），root transform 把 design 映射进 safe 区。
        /// ScreenToDesign 用同一公式逐项逆映射，保触摸↔渲染对齐。
        ///
        /// 前向映射（design→screen，组合 root transform + 正交相机）：
        ///   screen.x = rootPos.x + dx*sf + sw/2     （world.x = rootPos.x + dx*sf；screen.x = world.x + sw/2）
        ///   screen.y = rootPos.y - dy*sf + sh/2     （world.y = rootPos.y - dy*sf，y-flip；screen.y = world.y + sh/2）
        /// 令 offX = 设计 span 在屏幕的左边距（screen.x of design(0)），offYTop = span 顶边（screen.y of design(0)）：
        ///   offX   = area.x + (area.width  - dw*sf) * 0.5   （safe 区水平居中 rendered span dw*sf）
        ///   offYTop= area.y + area.height                  （Unity screen y 下原点，设计 y 上原点 → span 顶 = safe 区顶）
        ///   rootPos.x = offX   - sw/2     （令 screen.x of design(0) = offX = rootPos.x + sw/2）
        ///   rootPos.y = offYTop - sh/2     （令 screen.y of design(0) = offYTop = rootPos.y + sh/2）
        /// </summary>
        (float sf, Vector3 rootPos) ComputeRootTransform()
        {
            float sw = Screen.width, sh = Screen.height;
            Rect area = _safeArea ? Screen.safeArea : new Rect(0, 0, sw, sh);
            // 防御：safeArea 可能零宽高（编辑器未配屏）→ 退回全屏
            if (area.width <= 0f || area.height <= 0f) area = new Rect(0, 0, sw, sh);
            float dw = _designSize.x, dh = _designSize.y;
            // shrink-to-fit：取较小缩放比，保证完整可见 + 留白 letterbox。
            float sf = Mathf.Min(area.width / dw, area.height / dh);
            // 把设计的 rendered span（dw*sf × dh*sf）在 safe 区内居中。
            // offX = safe 左边 + 半（safe 宽 - rendered 宽）；span 顶 = safe 区顶（Unity screen y 下原点）。
            float offX = area.x + (area.width - dw * sf) * 0.5f;
            float offYTop = area.y + area.height;
            // world-root 位置：令 design(0,0) 渲染到 screen(offX, offYTop) [span 左上角，y 已 flip]。
            Vector3 rootPos = new Vector3(offX - sw * 0.5f, offYTop - sh * 0.5f, 0f);
            return (sf, rootPos);
        }

        void ConfigureTransforms()
        {
            float sw = Screen.width, sh = Screen.height;
            var (sf, rootPos) = ComputeRootTransform();

            transform.localScale = new Vector3(sf, -sf, sf);
            transform.localPosition = rootPos;

            if (_uiCamera != null)
            {
                _uiCamera.orthographic = true;
                _uiCamera.orthographicSize = sh / 2f;   // 不变（覆盖全屏，root 映射进 safe 区）
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

            // 输入采集 → set_input（tick 前——input 管线消费本帧输入产事件）。
            if (_inputCollector != null)
            {
                _inputCollector.Collect((System.IntPtr)_stage, _designSize, _safeArea);
                _inputCollector.CollectKeys((System.IntPtr)_stage);   // 键盘采集（tick 前）
                LoomInputCollector.CollectWheel(this);                 // 滚轮采集（tick 前）
            }

            // tick → build_blob 写入 Rust 拥有缓存。dt 累积进 time_s（双击窗口）；用
            // unscaledDeltaTime（照 fgui Time.unscaledTime，暂停不受影响）。
            Native.loomgui_stage_tick(_stage, Time.unscaledDeltaTime);

            // borrow_frame：返回 byte*（缓存首），写 nuint 长度。
            // 局部变量已在栈上固定，直接 & 取址传入（fixed 反而报 CS0213 "already fixed"）。
            nuint lenRaw = 0;
            byte* ptr = Native.loomgui_stage_borrow_frame(_stage, &lenRaw);
            int len = (int)lenRaw;
            if (ptr != null && len > 0)
            {
                // ArrayPool 租用（冷帧零 GC）。Rent 返回 ≥len，多余字节忽略。
                if (_frameBuf == null || _frameBuf.Length < len)
                {
                    if (_frameBuf != null) ArrayPool<byte>.Shared.Return(_frameBuf);
                    _frameBuf = ArrayPool<byte>.Shared.Rent(len);
                }
                Marshal.Copy((IntPtr)ptr, _frameBuf, 0, len);

                var blob = new FrameBlob(_frameBuf);
                _pool.Sync(blob, transform, _mm, _sprites, Texture2D.whiteTexture, _font);
                _nhm.Sync(blob);
            }

            // 事件派发（tick 后——borrow_events 读本帧 last_events，下 tick 失效）。
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
            _nhm?.Clear();
            _mm?.Clear();
            // v1.4-a T8：SpriteResolver 是纯缓存（Dictionary + List<SpriteAtlas>），无 UnityEngine.Object
            // 持有（SpriteAtlas/Sprite 由开发者 asset 持有，LoomStage 不销毁）。清缓存即可。
            _sprites?.Clear();
            // 归还 ArrayPool 租用的 _frameBuf。
            if (_frameBuf != null)
            {
                ArrayPool<byte>.Shared.Return(_frameBuf);
                _frameBuf = null;
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

        // Domain reload 保护（照 fgui Stage.cs:86）。SubsystemRegistration 在 Domain reload 时跑
        // （关闭 Domain Reload 仍跑——这正是本 hook 存在的根因：关 reload 时 C# 静态活过 Play，
        // 但 native 状态可能悬空）。
        //   1. Native.loomgui_shutdown() — native 全局态当前为空（Stage per-handle，stage_free drop），
        //      但 hook 必须接——引入 global texture/font registry 时此处自动清，无需再改接线。
        //      （注意：Font 的 Box::leak 是真泄漏，每次 Stage 创建 leak 一份字体字节——不可由 shutdown
        //      回收，需字体缓存化才能根治。×20 域重载测观察内存增长决定是否根治。）
        //   2. TextRasterizer.ResetStatic() — 清 C# 静态 s_fontVersion（atlas rebuild 计数器）。
        //      （MaterialManager/MirrorPool 都是 per-instance，随 MonoBehaviour OnDestroy 销毁，无 static。）
        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]
        static void ResetStatics()
        {
            Native.loomgui_shutdown();
            TextRasterizer.ResetStatic();
        }
    }
}
