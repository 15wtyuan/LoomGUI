using System.Runtime.InteropServices;
using UnityEngine;
using LoomGUI.Bindings;
#if ENABLE_INPUT_SYSTEM
using UnityEngine.InputSystem;
#endif

namespace LoomGUI
{
    /// v1c.3 输入采集：Unity 指针（鼠标+触摸）→ PointerEvent[] → loomgui_stage_set_input。
    /// screen→design 映射 + y-flip（Unity 左下原点 → LoomGUI 左上原点 design）。
    /// 兼容新旧输入系统：ENABLE_INPUT_SYSTEM 宏（Player Settings Active Input Handling=New/Both）走
    /// InputSystem API（Mouse.current / Touchscreen.current.touches），否则走旧 UnityEngine.Input。
    [ExecuteAlways]
    public unsafe class LoomInputCollector : MonoBehaviour
    {
        /// v1d.1：screen→design 映射，与 LoomStage.ComputeRootTransform 逐项逆（同一 sf 居中公式）。
        /// 前向（design→screen，见 ComputeRootTransform 注释）：
        ///   screen.x = offX    + dx*sf     其中 offX = area.x + (area.width  - dw*sf)*0.5
        ///   screen.y = offYTop - dy*sf     其中 offYTop = area.y + area.height
        /// 逆：
        ///   dx = (screen.x - offX)    / sf
        ///   dy = (offYTop - screen.y) / sf
        /// sf = min(area.w/dw, area.h/dh)【统一缩放，与渲染一致——修 v1c 渲染用 uniform sf / 输入用 per-axis stretch 的潜在错位】。
        /// useSafeArea=false 时 area 退回全屏（v1c 行为）。
        /// ponytail 验证：safe==全屏 + width-binding（sf=sw/dw）→ offX=0、offYTop=sh → dx=screen.x*dw/sw、dy=(sh-screen.y)*dw/sw ✓（v1c 式，但 y 也用 sf——见报告）
        public static Vector2 ScreenToDesign(Vector2 screen, Vector2Int screenSize, Vector2 rootSize, Rect area, bool useSafeArea)
        {
            float sw = screenSize.x > 0 ? screenSize.x : 1;
            float sh = screenSize.y > 0 ? screenSize.y : 1;
            Rect a = useSafeArea ? area : new Rect(0, 0, sw, sh);
            // 防御：safeArea 可能零宽高（编辑器未配屏）→ 退回全屏
            if (a.width <= 0f || a.height <= 0f) a = new Rect(0, 0, sw, sh);
            float dw = rootSize.x > 0 ? rootSize.x : 1;
            float dh = rootSize.y > 0 ? rootSize.y : 1;
            // 统一 shrink-to-fit 缩放（与 ComputeRootTransform 同一式）。
            float sf = Mathf.Min(a.width / dw, a.height / dh);
            sf = sf > 0 ? sf : 1f;   // 除零保护
            float offX = a.x + (a.width - dw * sf) * 0.5f;
            float offYTop = a.y + a.height;
            float dx = (screen.x - offX) / sf;
            float dy = (offYTop - screen.y) / sf;
            return new Vector2(dx, dy);
        }

        /// 采集本帧指针（鼠标+触摸）→ set_input。鼠标 touch_id=-1（slot0），触摸 touch_id=fingerId（slot1-4）。
        /// v1c.3：同帧共存（带触摸屏桌面）；EditMode 无 Touchscreen 跳过触摸。
        public void Collect(System.IntPtr stage, Vector2 rootSize, bool useSafeArea)
        {
            if (stage == System.IntPtr.Zero) return;
            var events = new System.Collections.Generic.List<Bindings.PointerEvent>();
            var screenSize = new Vector2Int(Screen.width, Screen.height);
            Rect safeArea = Screen.safeArea;

#if ENABLE_INPUT_SYSTEM
            // 鼠标（主指，touch_id=-1）
            if (Mouse.current != null)
            {
                var screen = Mouse.current.position.ReadValue();
                byte kind = 2;
                if (Mouse.current.leftButton.wasPressedThisFrame) kind = 0;
                else if (Mouse.current.leftButton.wasReleasedThisFrame) kind = 1;
                var d = ScreenToDesign(screen, screenSize, rootSize, safeArea, useSafeArea);
                events.Add(new Bindings.PointerEvent { kind = kind, button = 0, pad0 = 0, pad1 = 0, touch_id = -1, x = d.x, y = d.y });
            }
            // 触摸（多指）。TouchPhase 在 UnityEngine.InputSystem（非 LowLevel——坑：1.19 包 TouchPhase 不在 LowLevel）。
            if (Touchscreen.current != null)
            {
                foreach (var touch in Touchscreen.current.touches)
                {
                    if (touch == null) continue;
                    var phase = touch.phase.ReadValue();
                    if (phase == UnityEngine.InputSystem.TouchPhase.Stationary) continue;
                    byte kind = 2;
                    if (phase == UnityEngine.InputSystem.TouchPhase.Began) kind = 0;
                    else if (phase == UnityEngine.InputSystem.TouchPhase.Ended) kind = 1;
                    else if (phase == UnityEngine.InputSystem.TouchPhase.Canceled) kind = 3;   // v1c.4：Canceled
                    var screen = touch.position.ReadValue();
                    var d = ScreenToDesign(screen, screenSize, rootSize, safeArea, useSafeArea);
                    events.Add(new Bindings.PointerEvent { kind = kind, button = 0, pad0 = 0, pad1 = 0, touch_id = touch.touchId.ReadValue(), x = d.x, y = d.y });
                }
            }
#else
            // 旧输入系统
            var mscreen = Input.mousePosition;
            byte mkind = 2;
            if (Input.GetMouseButtonDown(0)) mkind = 0;
            else if (Input.GetMouseButtonUp(0)) mkind = 1;
            var md = ScreenToDesign(mscreen, screenSize, rootSize, safeArea, useSafeArea);
            events.Add(new Bindings.PointerEvent { kind = mkind, button = 0, pad0 = 0, pad1 = 0, touch_id = -1, x = md.x, y = md.y });
            foreach (var t in Input.touches)
            {
                if (t.phase == UnityEngine.TouchPhase.Stationary) continue;
                byte kind = 2;
                if (t.phase == UnityEngine.TouchPhase.Began) kind = 0;
                else if (t.phase == UnityEngine.TouchPhase.Ended) kind = 1;
                else if (t.phase == UnityEngine.TouchPhase.Canceled) kind = 3;   // v1c.4：Canceled
                var d = ScreenToDesign(t.position, screenSize, rootSize, safeArea, useSafeArea);
                events.Add(new Bindings.PointerEvent { kind = kind, button = 0, pad0 = 0, pad1 = 0, touch_id = t.fingerId, x = d.x, y = d.y });
            }
#endif
            if (events.Count == 0)
            {
                Native.loomgui_stage_set_input((Bindings.StageHandle*)stage, null, 0);
                return;
            }
            // csbindgen 生成的 set_input 取 PointerEvent*（raw 指针，非托管数组）+ nuint len。
            // events.ToArray() 是托管 PointerEvent[] —— 必须 fixed 钉住首元素取指针传入。
            var arr = events.ToArray();
            fixed (Bindings.PointerEvent* p = arr)
            {
                Native.loomgui_stage_set_input((Bindings.StageHandle*)stage, p, (nuint)arr.Length);
            }
        }

        /// v1d.2：采集本帧键盘 → set_key_input。KeyDown/Up 事件 + modifiers。
        /// 用 Input.GetKeyDown/Up（KeyCode 直对 core key_code，零转换；工程 Both 模式可用）——不像 Collect 双路径，
        /// 键盘无 InputSystem 特性需求。本帧无键事件 → set_key_input(null,0)（core 无键盘输入）。
        public void CollectKeys(System.IntPtr stage)
        {
            if (stage == System.IntPtr.Zero) return;
            var keys = new System.Collections.Generic.List<Bindings.KeyEvent>();
            byte mods = CurrentModifiers();
            // 键盘 down/up 用 Input.GetKeyDown/Up（KeyCode）——KeyCode 直接对应 core key_code=(uint)KeyCode，零转换。
            // 工程 Active Input Handling=Both（v1c.1 设），旧 Input API 可用；键盘无 InputSystem 特性需求，
            // 不走 Keyboard[Key]（要 InputSystem.Key 映射，过度复杂）。
            foreach (UnityEngine.KeyCode kc in KeyList)
            {
                bool down = UnityEngine.Input.GetKeyDown(kc);
                bool up = UnityEngine.Input.GetKeyUp(kc);
                if (down || up)
                    keys.Add(new Bindings.KeyEvent { key_code = (uint)kc, modifiers = mods, is_down = down, pad0 = 0, pad1 = 0 });
            }
            if (keys.Count == 0)
            {
                Native.loomgui_stage_set_key_input((Bindings.StageHandle*)stage, null, 0);
                return;
            }
            var arr = keys.ToArray();
            fixed (Bindings.KeyEvent* p = arr)
            {
                Native.loomgui_stage_set_key_input((Bindings.StageHandle*)stage, p, (nuint)arr.Length);
            }
        }

        /// v1d.5-T12：采集本帧滚轮 → set_wheel_input。tick 前调；累积式（多次调合并）。
        /// 新旧输入系统双路径（坑 28/45：滚轮用旧 Input.mouseScrollDelta 或新 Mouse.current.scroll）。
        /// 归一 delta → ±1/格：旧 Input.mouseScrollDelta 已 ≈ ±1/格；新系统 120 像素/格除 120。
        /// 鼠标不在 UI 上也可滚——hit test 由 Rust 侧做（只在悬停的 scroll 容器响应）。
        public static void CollectWheel(LoomStage stage)
        {
            if (stage == null || stage.StagePtr == System.IntPtr.Zero) return;

            float dy = 0f;
#if ENABLE_INPUT_SYSTEM
            var v = UnityEngine.InputSystem.Mouse.current?.scroll?.ReadValue() ?? Vector2.zero;
            dy = v.y / 120f;  // 归一：新系统 ~120 像素/格 → ±1/格
#else
            dy = Input.mouseScrollDelta.y;  // 旧系统已 ≈ ±1/格
#endif
            if (Mathf.Approximately(dy, 0f)) return;

            Vector2 screenPos;
#if ENABLE_INPUT_SYSTEM
            screenPos = UnityEngine.InputSystem.Mouse.current?.position?.ReadValue() ?? Vector2.zero;
#else
            screenPos = Input.mousePosition;
#endif

            var ss = new Vector2Int(Screen.width, Screen.height);
            Rect sa = Screen.safeArea;
            var pos = ScreenToDesign(screenPos, ss, stage.DesignSize, sa, stage.UseSafeArea);

            var ev = new Bindings.WheelEvent { x = pos.x, y = pos.y, delta_x = 0f, delta_y = dy };
            // 栈局部值类型直接 & 取址（CS0213：栈上已固定，无需 fixed）。
            Native.loomgui_stage_set_wheel_input((Bindings.StageHandle*)stage.StagePtr, &ev, 1);
        }

        /// 当前 modifiers 位掩码（bit0=shift/bit1=ctrl/bit2=alt）。core MOD_SHIFT/CTRL/ALT 同值。
        static byte CurrentModifiers()
        {
            byte m = 0;
#if ENABLE_INPUT_SYSTEM
            var kb = UnityEngine.InputSystem.Keyboard.current;
            if (kb == null) return 0;
            if (kb.leftShiftKey.isPressed || kb.rightShiftKey.isPressed) m |= 0x01;
            if (kb.leftCtrlKey.isPressed || kb.rightCtrlKey.isPressed) m |= 0x02;
            if (kb.leftAltKey.isPressed || kb.rightAltKey.isPressed) m |= 0x04;
#else
            if (UnityEngine.Input.GetKey(UnityEngine.KeyCode.LeftShift) || UnityEngine.Input.GetKey(UnityEngine.KeyCode.RightShift)) m |= 0x01;
            if (UnityEngine.Input.GetKey(UnityEngine.KeyCode.LeftControl) || UnityEngine.Input.GetKey(UnityEngine.KeyCode.RightControl)) m |= 0x02;
            if (UnityEngine.Input.GetKey(UnityEngine.KeyCode.LeftAlt) || UnityEngine.Input.GetKey(UnityEngine.KeyCode.RightAlt)) m |= 0x04;
#endif
            return m;
        }

        /// 采集的键白名单（Tab + 字母 + Enter/Space/Esc/方向 + 数字）。避免全 KeyCode 枚举遍历（数百个）开销。
        /// ponytail: 显式白名单而非全枚举——绝大多数键业务不关心，白名单够用且省 CPU。
        static readonly UnityEngine.KeyCode[] KeyList = {
            UnityEngine.KeyCode.Tab,
            UnityEngine.KeyCode.Return, UnityEngine.KeyCode.Space, UnityEngine.KeyCode.Escape,
            UnityEngine.KeyCode.LeftArrow, UnityEngine.KeyCode.RightArrow, UnityEngine.KeyCode.UpArrow, UnityEngine.KeyCode.DownArrow,
            UnityEngine.KeyCode.A, UnityEngine.KeyCode.B, UnityEngine.KeyCode.C, UnityEngine.KeyCode.D, UnityEngine.KeyCode.E,
            UnityEngine.KeyCode.F, UnityEngine.KeyCode.G, UnityEngine.KeyCode.H, UnityEngine.KeyCode.I, UnityEngine.KeyCode.J,
            UnityEngine.KeyCode.K, UnityEngine.KeyCode.L, UnityEngine.KeyCode.M, UnityEngine.KeyCode.N, UnityEngine.KeyCode.O,
            UnityEngine.KeyCode.P, UnityEngine.KeyCode.Q, UnityEngine.KeyCode.R, UnityEngine.KeyCode.S, UnityEngine.KeyCode.T,
            UnityEngine.KeyCode.U, UnityEngine.KeyCode.V, UnityEngine.KeyCode.W, UnityEngine.KeyCode.X, UnityEngine.KeyCode.Y, UnityEngine.KeyCode.Z,
            UnityEngine.KeyCode.Alpha0, UnityEngine.KeyCode.Alpha1, UnityEngine.KeyCode.Alpha2, UnityEngine.KeyCode.Alpha3, UnityEngine.KeyCode.Alpha4,
            UnityEngine.KeyCode.Alpha5, UnityEngine.KeyCode.Alpha6, UnityEngine.KeyCode.Alpha7, UnityEngine.KeyCode.Alpha8, UnityEngine.KeyCode.Alpha9,
        };
    }
}
