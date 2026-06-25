using UnityEngine;

namespace LoomGUI
{
    /// <summary>
    /// v1d.3-T11 PlayMode sample: transform demo + NativeHost + dump。
    /// 挂在与 LoomStage 同 GO，Start 里：
    ///   - BindNativeHost: 测试 cube 绑定到 HTML id="model-slot"
    ///   - DumpScene: 输出整树 JSON（含 id/classes/layout/world_matrix）
    ///
    /// 家里机验收：旋转/剪切/缩放视觉 + 命中 + NativeHost 跟随 + dump 日志。
    /// 配套 HTML/CSS 见 Samples/v1d3-transform-demo/。
    /// </summary>
    public class LoomTransformDemo : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[LoomTransformDemo] 未找到 LoomStage"); return; }
        }

        void Start()
        {
            if (_stage == null) return;

            // NativeHost: 挂测试 cube 到 model-slot
            var cube = GameObject.CreatePrimitive(PrimitiveType.Cube);
            cube.transform.localScale = Vector3.one * 50f;
            cube.name = "NativeHostCube";
            _stage.BindNativeHost("model-slot", cube);

            // dump 调试：整树 JSON（id/classes/layout/world_matrix）
            Debug.Log("[LoomGUI] scene tree:\n" + _stage.DumpScene());
        }
    }
}
