using UnityEngine;

namespace LoomGUI
{
    // v1 showcase driver: nav 跳转 + 启动错峰入场 + 交互灯阵（T7）+ tween 演示（T8）。
    public unsafe class LoomShowcaseDriver : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;
        [SerializeField] float[] _sectionY = { 0f, 700f, 1500f, 2300f, 3000f, 3800f, 4500f, 5200f }; // 设计期累积高度（sec-1..sec-8 顶部 y）

        uint _scrollNode = uint.MaxValue;
        uint[] _navNodes = new uint[8];

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[Showcase] 无 LoomStage"); return; }
        }

        void Start()
        {
            _scrollNode = _stage.FindNodeById("main-scroll");
            for (int i = 0; i < 8; i++)
            {
                _navNodes[i] = _stage.FindNodeById("nav-" + (i + 1));
                if (_navNodes[i] != uint.MaxValue)
                    _stage.EventHandler.AddListener(_navNodes[i], EventType.Click, OnNavClick);
            }
            Debug.Log($"[Showcase] scroll={_scrollNode} nav0={_navNodes[0]}（点 nav 跳区）");
            StaggeredEntrance();
        }

        // design-taste §4: 启动错峰入场。各 sec 卡 tween opacity 0→1 + delay 递增。
        // 注：HTML 各 sec 初始 opacity:0（见 style.css .sec），靠 tween 拉亮；同时验证 §7.1 opacity + §7.3 delay。
        void StaggeredEntrance()
        {
            for (int i = 0; i < 8; i++)
            {
                uint node = _stage.FindNodeById("sec-" + (i + 1));
                if (node == uint.MaxValue) continue;
                _stage.Tween(node, TweenProp.Opacity,
                    new float[] { 0f, 0, 0, 0 }, new float[] { 1f, 0, 0, 0 },
                    0.4f, Ease.CubicOut, i * 0.04f, 0);
            }
        }

        void OnNavClick(EventContext ctx)
        {
            if (_scrollNode == uint.MaxValue) return;
            for (int i = 0; i < 8; i++)
            {
                if (ctx.target == _navNodes[i])
                {
                    _stage.SetScrollPos(_scrollNode, 0f, _sectionY[i], true);
                    return;
                }
            }
        }
    }
}
