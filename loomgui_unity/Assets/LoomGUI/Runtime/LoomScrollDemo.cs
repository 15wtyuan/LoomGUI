using UnityEngine;
using System.Collections;

namespace LoomGUI
{
    /// v1d.5-T12：ScrollPane PlayMode 验收 sample。
    ///
    /// 挂在与 LoomStage 同 GO；stage 的 _html/_css 设置为下方 sample（见注释）。
    /// 垂直长列表（overflow-y:scroll + 固定高容器 + 多子项）演示：
    ///   拖拽跟手 / Up 惯性 / 边界回弹 / 滚轮 / grip / 嵌套轴锁 / scroll-vs-draggable / 编程 SetScrollPos。
    ///
    /// sample HTML（贴到 LoomStage._html）：
    /// <div id="scroll-area" class="scroll-container">
    ///   <div class="item">Item 0</div>
    ///   <div class="item">Item 1</div>
    ///   ... (repeat to Item 49, 50 items total)
    ///   <div id="drag-me" class="item-drag" draggable="true">Drag Me (scroll-vs-drag)</div>
    ///   <div class="nested-hscroll">
    ///     <div class="hitem">H1</div><div class="hitem">H2</div>...（10 个宽子项，验轴锁）
    ///   </div>
    /// </div>
    ///
    /// sample CSS（贴到 LoomStage._css）：
    /// .scroll-container{width:600px;height:400px;overflow-y:scroll;background-color:#1a1a2e;border:2px solid #999;}
    /// .item{width:560px;height:44px;margin:4px 20px;background-color:#16213e;color:#eee;font-size:16px;display:flex;align-items:center;padding-left:12px;}
    /// .item-drag{width:560px;height:44px;margin:4px 20px;background-color:#0f3460;color:#e94560;font-size:16px;display:flex;align-items:center;justify-content:center;}
    /// .nested-hscroll{width:560px;height:56px;margin:4px 20px;overflow-x:scroll;overflow-y:hidden;white-space:nowrap;background-color:#533483;}
    /// .hitem{display:inline-block;width:140px;height:48px;margin:4px;background-color:#7b2ff7;color:#fff;font-size:14px;text-align:center;line-height:48px;}
    public class LoomScrollDemo : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;
        [SerializeField] string _scrollContainerId = "scroll-area";

        // 50 项 vertical list（每个 item 44px + 8px margin = 52px → 2600px content > 400px viewport）
        const int ItemCount = 50;

        /// sample HTML（共 50 item + 1 draggable + 1 嵌套 horizontal scroll）。贴到 LoomStage._html。
        public static string SampleHtml
        {
            get
            {
                var sb = new System.Text.StringBuilder(1 << 12);
                sb.Append("<div id=\"scroll-area\" class=\"scroll-container\">");
                for (int i = 0; i < ItemCount; i++)
                    sb.Append("<div class=\"item\">Item ").Append(i).Append("</div>");
                sb.Append("<div id=\"drag-me\" class=\"item-drag\" draggable=\"true\">Drag Me (scroll-vs-drag)</div>");
                sb.Append("<div class=\"nested-hscroll\">");
                for (int i = 0; i < 10; i++)
                    sb.Append("<div class=\"hitem\">H").Append(i).Append("</div>");
                sb.Append("</div></div>");
                return sb.ToString();
            }
        }

        /// sample CSS。贴到 LoomStage._css。
        public static string SampleCss =>
            ".scroll-container{width:600px;height:400px;overflow-y:scroll;background-color:#1a1a2e;border:2px solid #999;}"
            + ".item{width:560px;height:44px;margin:4px 20px;background-color:#16213e;color:#eee;font-size:16px;display:flex;align-items:center;padding-left:12px;}"
            + ".item-drag{width:560px;height:44px;margin:4px 20px;background-color:#0f3460;color:#e94560;font-size:16px;display:flex;align-items:center;justify-content:center;}"
            + ".nested-hscroll{width:560px;height:56px;margin:4px 20px;overflow-x:scroll;overflow-y:hidden;white-space:nowrap;background-color:#533483;}"
            + ".hitem{display:inline-block;width:140px;height:48px;margin:4px;background-color:#7b2ff7;color:#fff;font-size:14px;text-align:center;line-height:48px;}";

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[scroll-demo] 未找到 LoomStage"); return; }
        }

        IEnumerator Start()
        {
            // 等一帧让 stage Awake 完成 load
            yield return null;

            uint scrollNode = _stage.FindNodeById(_scrollContainerId);
            if (scrollNode == uint.MaxValue)
            {
                Debug.LogWarning($"[scroll-demo] 未找到 id='{_scrollContainerId}'（HTML 含此 id？）");
                yield break;
            }

            Debug.Log($"[scroll-demo] scroll container node={scrollNode}, {ItemCount} items + drag + nested hscroll ready");

            // 演示 1：SetScrollPos animated scroll to y=300（2s 后）
            yield return new WaitForSeconds(2f);
            Debug.Log("[scroll-demo] SetScrollPos → y=300 (animated cubic-out)");
            _stage.SetScrollPos(scrollNode, 0f, 300f, animated: true);

            // 演示 2：SetScrollPos instant snap to y=800（再过 3s）
            yield return new WaitForSeconds(3f);
            Debug.Log("[scroll-demo] SetScrollPos → y=800 (instant)");
            _stage.SetScrollPos(scrollNode, 0f, 800f, animated: false);

            // 演示 3：SetScrollPos animated back to y=0（零回归，再过 3s）
            yield return new WaitForSeconds(3f);
            Debug.Log("[scroll-demo] SetScrollPos → y=0 (animated, 零回归)");
            _stage.SetScrollPos(scrollNode, 0f, 0f, animated: true);
        }
    }
}
