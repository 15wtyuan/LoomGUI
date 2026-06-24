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
    }
}
