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
        /// v1d.1：screen→design 映射 + y-flip + safe-area 偏移（与 LoomStage.ComputeRootTransform 对齐）。
        /// 简化用 safe 区直接映射（safe 区 = root 渲染区，二者一一对应）：
        ///   design_x = (screen_x - area.x) / area.width * rootW
        ///   design_y = rootH - (screen_y - area.y) / area.height * rootH
        /// useSafeArea=false 时 area 退回全屏（v1c 行为）；safe==全屏时 area.x=area.y=0 → 等价原式 ✓
        public static Vector2 ScreenToDesign(Vector2 screen, Vector2Int screenSize, Vector2 rootSize, Rect area, bool useSafeArea)
        {
            Rect a = useSafeArea ? area : new Rect(0, 0, screenSize.x, screenSize.y);
            // 除零保护：area/rootSize 退化为 0（EditMode 未配屏 / 桌面 minimize）。
            float aw = a.width > 0 ? a.width : 1;
            float ah = a.height > 0 ? a.height : 1;
            float rx = rootSize.x > 0 ? rootSize.x : 1;
            float ry = rootSize.y > 0 ? rootSize.y : 1;
            float dx = (screen.x - a.x) / aw * rx;
            float dy = ry - ((screen.y - a.y) / ah * ry);
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
