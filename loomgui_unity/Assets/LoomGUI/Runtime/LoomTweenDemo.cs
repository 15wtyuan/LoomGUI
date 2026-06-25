using UnityEngine;

namespace LoomGUI
{
    /// v1d.4 PlayMode sample：fade-in + pop-in(BackOut) + onComplete 链式（tag 识别）。
    /// 挂在 LoomStage 同 GO（Awake 取 stage），或 Inspector 指定 _stage。
    public unsafe class LoomTweenDemo : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;
        // 要动的节点 CSS id（demo html 须有对应 id 属性）。
        [SerializeField] string _targetId = "popup";
        // 链式 tag 常量。
        const uint TagFadeIn = 1;
        const uint TagPopIn = 2;

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[LoomTweenDemo] 无 LoomStage"); return; }
        }

        /// 业务在合适时机调（如开面板按钮 click）。Start 里 demo 触发一次。
        void Start()
        {
            uint node = _stage.FindNodeById(_targetId);
            if (node == uint.MaxValue) { Debug.LogError($"[LoomTweenDemo] id '{_targetId}' 未找到"); return; }
            // fade-in（opacity 0→1, 0.3s Linear）+ pop（scale 0.8→1.0, 0.3s BackOut）并发（不同通道）
            _stage.Tween(node, TweenProp.Opacity, new float[] { 0f, 0, 0, 0 }, new float[] { 1f, 0, 0, 0 }, 0.3f, Ease.Linear, 0f, TagFadeIn);
            _stage.Tween(node, TweenProp.Scale, new float[] { 0.8f, 0.8f, 0, 0 }, new float[] { 1f, 1f, 0, 0 }, 0.3f, Ease.BackOut, 0f, TagPopIn);
            // 监听 fade 完成（按 tag 链式触发下一段——demo 仅 log）
            _stage.EventHandler.AddListener(node, EventType.TweenComplete, OnTweenComplete);
        }

        static void OnTweenComplete(EventContext ctx)
        {
            // prop = ctx.clickCount，tag = ctx.touchId
            Debug.Log($"[LoomTweenDemo] tween complete: tag={ctx.touchId} prop={ctx.clickCount}");
        }
    }
}
