using System.Runtime.InteropServices;
using UnityEngine;
using LoomGUI.Bindings;

namespace LoomGUI
{
    /// v1c.1 输入采集：Unity 旧输入系统 → PointerEvent[] → loomgui_stage_set_input。
    /// screen→design 映射 + y-flip（Unity 左下原点 → LoomGUI 左上原点 design）。
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

            var screen = Input.mousePosition;
            var design = ScreenToDesign(screen, new Vector2Int(Screen.width, Screen.height), rootSize);

            // v1c.1 单事件/帧：Down/Up 边沿优先；其余 Move。kind 与 Rust PointerKind 一致（0/1/2）。
            byte kind = 2; // PointerKind::Move
            if (Input.GetMouseButtonDown(0)) kind = 0;       // PointerKind::Down
            else if (Input.GetMouseButtonUp(0)) kind = 1;    // PointerKind::Up

            var ev = new Bindings.PointerEvent { kind = kind, x = design.x, y = design.y, button = 0 };
            // set_input 走 csbindgen 生成的 Native.loomgui_stage_set_input（StageHandle*）。
            // LoomStage 持 _stage 为 StageHandle*，这里经 (StageHandle*) 强转传入。
            Native.loomgui_stage_set_input((Bindings.StageHandle*)stage, &ev, 1);
        }
    }
}
