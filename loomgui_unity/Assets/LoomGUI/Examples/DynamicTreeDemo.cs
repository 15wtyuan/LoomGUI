using UnityEngine;

namespace LoomGUI
{
    /// <summary>
    /// T8 动态建树演示（spec §11.2 验收场景）。挂在带 LoomStage 的 GameObject 上，
    /// LoomStage.Awake 先用 inline _html/_css 建一个最小 scene（单空根 div，满足 create_root 等
    /// 动态 API 的 scene 前置），本脚本 Start 再用 9 个动态 API 在其上纯动态建 UI：
    ///
    /// 场景1 层级骨架：CreateRoot 建 stage 根（覆盖默认空根）+ CreateNode 建 layer 容器。
    /// 场景2 锚点挂载：CreateNode 建 panel + title(span) + icon(img)，AppendChild 挂载，
    ///                 SetText 填标题、SetSrc 填图源（无 atlas 时 fallback 白占位，不阻塞）。
    /// 场景3 set_style 改样式：SetStyle 改 panel 底色（白→灰），下帧 rematch 生效。
    ///
    /// 用返回的 NodeId 句柄（勿硬编码 0——slotmap idx 从 1 起，首节点 NodeId 非 0）。
    /// 失败（0xFFFF_FFFF / -1）记 LogError 但不中断（便于部分验收）。
    /// </summary>
    public unsafe class DynamicTreeDemo : MonoBehaviour
    {
        [SerializeField] LoomStage _stage;

        // 场景3 触发：按 Space 切 panel 样式（验 SetStyle 增量改 base_style + 下帧 rematch）。
        [SerializeField] bool _toggleStyleOnSpace = true;

        uint _panel = uint.MaxValue;
        bool _gray;

        void Awake()
        {
            if (_stage == null) _stage = GetComponent<LoomStage>();
            if (_stage == null) { Debug.LogError("[DynamicTreeDemo] 缺 LoomStage"); enabled = false; }
        }

        void Start()
        {
            if (_stage == null) return;

            // 场景1：层级骨架。
            // CreateRoot 建动态根（stage 根，覆盖 inline 建的空根——两 root 并存，动态根作为 UI 容器）。
            uint root = _stage.CreateRoot("div",
                "width:1920px;height:1080px;background:#1a1d2e;overflow:hidden");
            if (root == uint.MaxValue) { Debug.LogError("[DynamicTreeDemo] CreateRoot 失败"); return; }
            Debug.Log($"[DynamicTreeDemo] 场景1 CreateRoot → node {root}");

            // 建 layer 容器（挂 root 下，作为 panel 的父层）。
            uint layer = _stage.CreateNode("div",
                "width:1920px;height:1080px;display:flex;flex-direction:column;align-items:center;justify-content:center");
            if (layer == uint.MaxValue) { Debug.LogError("[DynamicTreeDemo] CreateNode layer 失败"); return; }
            Check(_stage.AppendChild(root, layer), "AppendChild(layer)");

            // 场景2：锚点挂载（panel + title + icon，全动态建 + 挂载）。
            _panel = _stage.CreateNode("div",
                "width:400px;height:300px;background:#ffffff;overflow:hidden;display:flex;flex-direction:column;align-items:center");
            if (_panel == uint.MaxValue) { Debug.LogError("[DynamicTreeDemo] CreateNode panel 失败"); return; }
            Check(_stage.AppendChild(layer, _panel), "AppendChild(panel)");

            // 标题 span（Text 节点）。
            uint title = _stage.CreateNode("span", "font-size:24px;color:#000000;margin:10px");
            if (title == uint.MaxValue) { Debug.LogError("[DynamicTreeDemo] CreateNode title 失败"); return; }
            Check(_stage.AppendChild(_panel, title), "AppendChild(title)");
            Check(_stage.SetText(title, "背包"), "SetText(title)");

            // 图标 img（Image 节点；无 atlas 时 fallback 白占位，不阻塞渲染）。
            uint icon = _stage.CreateNode("img", "width:64px;height:64px;margin:20px");
            if (icon == uint.MaxValue) { Debug.LogError("[DynamicTreeDemo] CreateNode icon 失败"); return; }
            Check(_stage.AppendChild(_panel, icon), "AppendChild(icon)");
            Check(_stage.SetSrc(icon, "item_001.png"), "SetSrc(icon)");

            // insert_before 演示：建第二行图标，用 InsertBefore 插到 title 之前（验子序 API）。
            uint icon2 = _stage.CreateNode("img", "width:48px;height:48px;margin:8px");
            if (icon2 != uint.MaxValue)
            {
                Check(_stage.InsertBefore(_panel, icon2, title), "InsertBefore(icon2 before title)");
                Check(_stage.SetSrc(icon2, "item_002.png"), "SetSrc(icon2)");
            }

            Debug.Log($"[DynamicTreeDemo] 场景2 挂载完成 panel={_panel} title={title} icon={icon} icon2={icon2}");

            // 场景3：SetStyle 改样式。初始白底（CreateNode 的 css），按 Space 切灰底/白底。
            // 下帧 rematch 从 base_style 重算 → 渲染自动生效。
            Debug.Log("[DynamicTreeDemo] 场景3 按 Space 切换 panel 底色（SetStyle 增量改样式）");
        }

        void Update()
        {
            if (!_toggleStyleOnSpace || _panel == uint.MaxValue) return;
            if (Input.GetKeyDown(KeyCode.Space))
            {
                _gray = !_gray;
                string css = _gray ? "background:#eeeeee" : "background:#ffffff";
                Check(_stage.SetStyle(_panel, css), $"SetStyle({css})");
                Debug.Log($"[DynamicTreeDemo] SetStyle → {_panel} {(css)}");
            }
        }

        static void Check(int r, string label)
        {
            if (r != 0) Debug.LogError($"[DynamicTreeDemo] {label} 失败 → {r}");
        }
    }
}
