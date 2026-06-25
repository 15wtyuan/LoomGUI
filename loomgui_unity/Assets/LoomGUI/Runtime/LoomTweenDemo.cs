using UnityEngine;

namespace LoomGUI
{
    /// v1d.4 PlayMode sample：fade-in + pop-in(BackOut) + onComplete（tag 识别）。
    /// 点 OnGUI 按钮（或 Space）触发——避免 Start 首帧 dt spike（Unity PlayMode 首帧
    /// unscaledDeltaTime 可达数秒，会让 tween 瞬间 complete 写末值、无渐变）。
    public unsafe class LoomTweenDemo : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;
        [SerializeField] string _targetId = "popup";
        const uint TagFadeIn = 1;
        const uint TagPopIn = 2;

        uint _popupNode = uint.MaxValue;

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[LoomTweenDemo] 无 LoomStage"); return; }
        }

        void Start()
        {
            _popupNode = _stage.FindNodeById(_targetId);
            Debug.Log($"[LoomTweenDemo] FindNodeById('{_targetId}')={_popupNode}（点按钮或 Space 触发动画）");
            if (_popupNode != uint.MaxValue)
                _stage.EventHandler.AddListener(_popupNode, EventType.TweenComplete, OnTweenComplete);
        }

        void OnGUI()
        {
            if (_popupNode == uint.MaxValue) return;
            if (GUI.Button(new Rect(10, 10, 240, 36), "播放 tween（或按 Space）"))
                PlayTween();
        }

        void Update()
        {
            if (_popupNode == uint.MaxValue) return;
            if (Input.GetKeyDown(KeyCode.Space))
                PlayTween();
        }

        /// 触发 fade-in + pop-in。ClearAnim 重置到 CSS 初始，可重复点。
        void PlayTween()
        {
            _stage.ClearAnim(_popupNode);
            _stage.Tween(_popupNode, TweenProp.Opacity, new float[] { 0f, 0, 0, 0 }, new float[] { 1f, 0, 0, 0 }, 1.5f, Ease.Linear, 0f, TagFadeIn);
            _stage.Tween(_popupNode, TweenProp.Scale, new float[] { 0.8f, 0.8f, 0, 0 }, new float[] { 1f, 1f, 0, 0 }, 1.5f, Ease.BackOut, 0f, TagPopIn);
        }

        static void OnTweenComplete(EventContext ctx)
        {
            // prop = ctx.clickCount，tag = ctx.touchId
            Debug.Log($"[LoomTweenDemo] tween complete: tag={ctx.touchId} prop={ctx.clickCount}");
        }
    }
}
