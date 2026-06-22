using System.Runtime.InteropServices;
using UnityEngine;
using LoomGUI.Bindings;
#if ENABLE_INPUT_SYSTEM
using UnityEngine.InputSystem;
#endif

namespace LoomGUI
{
    /// v1c.1 输入采集：Unity 指针 → PointerEvent[] → loomgui_stage_set_input。
    /// screen→design 映射 + y-flip（Unity 左下原点 → LoomGUI 左上原点 design）。
    /// 兼容新旧输入系统：ENABLE_INPUT_SYSTEM 宏（Player Settings Active Input Handling=New/Both）走
    /// InputSystem API（Mouse.current），否则走旧 UnityEngine.Input。工程切了 Input System package 故用新路径。
    [ExecuteAlways]
    public unsafe class LoomInputCollector : MonoBehaviour
    {
        /// screen→design 映射 + y-flip（Unity 左下原点 → LoomGUI 左上原点 design）。
        /// design_x = screen_x / screen_w * root_w
        /// design_y = root_h - (screen_y / screen_h * root_h)
        public static Vector2 ScreenToDesign(Vector2 screen, Vector2Int screenSize, Vector2 rootSize)
        {
            // 除零保护：screenSize/rootSize 退化为 0（EditMode 未配屏 / 桌面 minimize）。
            float sx = screenSize.x > 0 ? screenSize.x : 1;
            float sy = screenSize.y > 0 ? screenSize.y : 1;
            float rx = rootSize.x > 0 ? rootSize.x : 1;
            float ry = rootSize.y > 0 ? rootSize.y : 1;
            float dx = screen.x / sx * rx;
            float dy = ry - (screen.y / sy * ry);
            return new Vector2(dx, dy);
        }

        /// 采集本帧指针 → set_input。挂 LoomStage 的 _stage 句柄。
        /// v1c.1 单指针：每帧产 1 个事件——Down/Up 边沿优先（同帧 Down+Move 简化为单事件，够用）。
        public void Collect(System.IntPtr stage, Vector2 rootSize)
        {
            if (stage == System.IntPtr.Zero) return;
            Vector2 screen;
            byte kind = 2; // PointerKind::Move
#if ENABLE_INPUT_SYSTEM
            // 新输入系统：Mouse.current.position（左下原点 screen 像素，同旧 Input.mousePosition 语义）。
            // EditMode 无 Mouse.current（null）→ 跳过采集（不产事件）。
            if (Mouse.current == null) return;
            screen = Mouse.current.position.ReadValue();
            if (Mouse.current.leftButton.wasPressedThisFrame) kind = 0;       // PointerKind::Down
            else if (Mouse.current.leftButton.wasReleasedThisFrame) kind = 1; // PointerKind::Up
#else
            // 旧输入系统：Input.mousePosition（左下原点）。
            screen = Input.mousePosition;
            if (Input.GetMouseButtonDown(0)) kind = 0;       // PointerKind::Down
            else if (Input.GetMouseButtonUp(0)) kind = 1;    // PointerKind::Up
#endif

            var design = ScreenToDesign(screen, new Vector2Int(Screen.width, Screen.height), rootSize);
            var ev = new Bindings.PointerEvent { kind = kind, x = design.x, y = design.y, button = 0 };
            // set_input 走 csbindgen 生成的 Native.loomgui_stage_set_input（StageHandle*）。
            // LoomStage 持 _stage 为 StageHandle*，这里经 (StageHandle*) 强转传入。
            Native.loomgui_stage_set_input((Bindings.StageHandle*)stage, &ev, 1);
        }
    }
}
