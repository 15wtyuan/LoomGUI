using UnityEngine;

namespace LoomGUI
{
    /// v1d interact sample 的 C# driver：演示全事件谱。
    /// v1c.1 disabled + v1c.2 capture/bubble/hover + v1c.3 capture touch + v1d.1 drag/longpress + v1d.2 focus/key。
    ///
    /// 挂在与 LoomStage 同 GO（_stage 引用），Awake 取 EventHandler 注册 listener。
    /// 全部 nodeId 走 FindNodeById（HTML id 属性），不依赖 build 序——加元素不破 id。
    /// sample id：btn_disabled / drag(draggable) / outer / foc1(tabindex=1) / foc2(=2) / foc3(=0)。
    public class LoomInteractDemo : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[demo] 未找到 LoomStage"); return; }
            RegisterListeners();
        }

        void RegisterListeners()
        {
            var h = _stage.EventHandler;

            uint disabledId = _stage.FindNodeById("btn_disabled");
            uint dragId     = _stage.FindNodeById("drag");
            uint outerId    = _stage.FindNodeById("outer");
            uint foc1Id     = _stage.FindNodeById("foc1");

            // ===== v1c.1 disabled：find by id → set Node.disabled（「禁用按钮」真禁用：:disabled + 抑制 active/click/longpress）=====
            if (disabledId != uint.MaxValue) _stage.SetNodeDisabled(disabledId, true);
            else Debug.LogWarning("[demo] find btn_disabled 失败——pkg 未含 id？");

            // ===== v1c.2 capture/bubble 演示：foc1 click → outer capture（先）+ foc1 target StopPropagation → outer bubble 不收 =====
            if (outerId != uint.MaxValue)
                h.AddCapture(outerId, EventType.Click, ctx =>
                    Debug.Log($"[v1c.2] outer CAPTURE click target={ctx.target} phase={ctx.phase}"));
            if (foc1Id != uint.MaxValue)
                h.AddListener(foc1Id, EventType.Click, ctx =>
                {
                    Debug.Log($"[v1c.2] foc1 click target/bubble → StopPropagation");
                    ctx.StopPropagation();   // outer bubble 回调不触发（capture 仍跑）
                });
            if (outerId != uint.MaxValue)
                h.AddListener(outerId, EventType.Click, ctx =>
                    Debug.Log($"[v1c.2] outer BUBBLE click（foc1 stop 时不触发）"));

            // ===== v1c.3 capture touch：foc1 Down → CaptureTouch → 手指移出仍收 Move =====
            if (foc1Id != uint.MaxValue)
            {
                h.AddListener(foc1Id, EventType.Down, ctx =>
                {
                    ctx.CaptureTouch();
                    Debug.Log($"[v1c.3] foc1 Down touch={ctx.touchId} → capture");
                });
                h.AddListener(foc1Id, EventType.Move, ctx =>
                    Debug.Log($"[v1c.3] foc1 Move (capture 中) pos=({ctx.x},{ctx.y})"));
            }

            // ===== v1d.1 drag（opt-in：仅 draggable=true 的 drag 元素）+ longpress（universal）=====
            if (dragId != uint.MaxValue)
            {
                h.AddListener(dragId, EventType.DragStart, ctx => Debug.Log($"[v1d.1] drag Start pos=({ctx.x},{ctx.y}) touch={ctx.touchId}（drag 起后 Up 无 click）"));
                h.AddListener(dragId, EventType.DragMove,  ctx => Debug.Log($"[v1d.1] drag Move pos=({ctx.x},{ctx.y})"));
                h.AddListener(dragId, EventType.DragEnd,   ctx => Debug.Log($"[v1d.1] drag End pos=({ctx.x},{ctx.y})"));
                // longpress universal：drag 节点按住 1.5s 发一次（独立 click——松手仍 Click）
                h.AddListener(dragId, EventType.LongPress, ctx => Debug.Log($"[v1d.1] longpress fired node={ctx.target}"));
            }

            // ===== v1d.2 focus（tabindex opt-in）+ keydown：foc1/2/3 都注册 =====
            foreach (var idName in new[] { "foc1", "foc2", "foc3" })
            {
                string name = idName;   // 闭包捕获局部拷贝
                uint id = _stage.FindNodeById(name);
                if (id == uint.MaxValue) continue;
                h.AddListener(id, EventType.FocusIn,  ctx => Debug.Log($"[v1d.2] focus IN  {name}({ctx.target})"));
                h.AddListener(id, EventType.FocusOut, ctx => Debug.Log($"[v1d.2] focus OUT {name}({ctx.target})"));
                h.AddListener(id, EventType.KeyDown,  ctx => Debug.Log($"[v1d.2] key down {name} code={ctx.keyCode} mod={ctx.modifiers}"));
            }

            // ===== v1c.2 hover 祖先链：foc hover → outer 也 RollOver（祖先链 diff）=====
            if (outerId != uint.MaxValue)
            {
                h.AddListener(outerId, EventType.RollOver, ctx => Debug.Log($"[hover] outer RollOver（后代 hover）"));
                h.AddListener(outerId, EventType.RollOut,  ctx => Debug.Log($"[hover] outer RollOut"));
            }
        }
    }
}
