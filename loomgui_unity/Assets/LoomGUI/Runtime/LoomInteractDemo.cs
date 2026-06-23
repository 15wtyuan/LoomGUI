using UnityEngine;

namespace LoomGUI
{
    /// v1c.2-T5 interact sample 的 C# driver：演示事件冒泡 / stopPropagation / capture 三阶段。
    /// 挂在与 LoomStage 同 GO（或任意 GO 上引用 LoomStage）——Awake 取 stage.EventHandler 注册 listener。
    ///
    /// nodeId 按 build 序（scene/node.rs build 顺序）：
    ///   root=0, btn1=1, btn2=2, btn_disabled=3, outer=4, inner btn=5
    /// **注：此 id 为推断序——家里机 PlayMode 调试确认实际 id（可临时在 listener 里打
    ///   ctx.target / ctx.currentTarget 对比命中节点）**。若序偏移，按实际调整常量。
    ///
    /// 演示：
    ///   - outer capture（先于 btn target/bubble）
    ///   - btn click + StopPropagation（btn stop → outer bubble 不收）
    ///   - outer bubble（btn 未 stop 时收；本 sample btn stop，故 outer bubble 回调不触发——
    ///     注释保留供调试：取消 btn 的 StopPropagation 即见 outer bubble 收到）
    public class LoomInteractDemo : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;

        // nodeId 推断序（家里机 PlayMode 调试确认实际 id）
        const uint OuterId = 4u;
        const uint BtnId = 5u;

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null)
            {
                Debug.LogError("[LoomInteractDemo] 未找到 LoomStage");
                return;
            }
            RegisterListeners();
        }

        void RegisterListeners()
        {
            var h = _stage.EventHandler;

            // capture：outer 在 capture 阶段先收（root→target 反向，全跑，stop 不阻断 capture）
            h.AddCapture(OuterId, EventType.Click, ctx =>
                Debug.Log($"[interact] outer capture click target={ctx.target} cur={ctx.currentTarget} phase={ctx.phase}"));

            // btn target/bubble：click + stop → outer bubble 不收（capture 仍跑）
            h.AddListener(BtnId, EventType.Click, ctx =>
            {
                Debug.Log($"[interact] btn click target={ctx.target} cur={ctx.currentTarget} phase={ctx.phase}");
                ctx.StopPropagation();   // 演示止冒泡：outer bubble 回调不触发
            });

            // outer bubble：btn stop 时此处不触发（保留验证 stop 生效）
            h.AddListener(OuterId, EventType.Click, ctx =>
                Debug.Log($"[interact] outer bubble click target={ctx.target} cur={ctx.currentTarget} phase={ctx.phase}"));

            // v1c.3 capture demo：Down 在 btn 时 CaptureTouch → 手指移出仍收 Move（核心 add_touch_monitor）
            h.AddListener(BtnId, EventType.Down, ctx => {
                ctx.CaptureTouch();
                Debug.Log($"[interact] btn Down touch={ctx.touchId} → capture");
            });
            h.AddListener(BtnId, EventType.Move, ctx => {
                Debug.Log($"[interact] btn Move (capture 中) touch={ctx.touchId} pos=({ctx.x},{ctx.y})");
            });

            // hover 祖先链演示：outer RollOver（btn hover 时 outer 也 RollOver——v1c.2 祖先链 diff）
            h.AddListener(OuterId, EventType.RollOver, ctx =>
                Debug.Log($"[interact] outer RollOver cur={ctx.currentTarget}"));
            h.AddListener(OuterId, EventType.RollOut, ctx =>
                Debug.Log($"[interact] outer RollOut cur={ctx.currentTarget}"));
            h.AddListener(BtnId, EventType.RollOver, ctx =>
                Debug.Log($"[interact] btn RollOver cur={ctx.currentTarget}"));
            h.AddListener(BtnId, EventType.RollOut, ctx =>
                Debug.Log($"[interact] btn RollOut cur={ctx.currentTarget}"));
        }
    }
}
