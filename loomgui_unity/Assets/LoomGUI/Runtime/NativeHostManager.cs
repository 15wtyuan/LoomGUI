using System.Collections.Generic;
using UnityEngine;

namespace LoomGUI
{
    /// <summary>
    /// NativeHost-lite（spec §3.7）：外部 GO 跟随 UI 节点 world transform + 显隐 + 排序。
    /// core 零改动，复用既有 loomgui_stage_find_node_by_id。
    /// </summary>
    internal sealed class NativeHostManager
    {
        private readonly Dictionary<uint, GameObject> _bindings = new();
        private Transform _root;
        // 本帧已同步的 node_id 集合（"node 消失 → SetActive(false)"）。
        private readonly HashSet<uint> _seenThisFrame = new();

        public void Init(Transform root) { _root = root; }

        public void Bind(uint nodeId, GameObject go)
        {
            if (go == null) return;
            Unbind(nodeId);
            go.transform.SetParent(_root, false);
            _bindings[nodeId] = go;
        }

        public void Unbind(uint nodeId)
        {
            if (_bindings.TryGetValue(nodeId, out var go))
            {
                go.SetActive(false);
                _bindings.Remove(nodeId);
            }
        }

        public void Clear()
        {
            foreach (var kv in _bindings)
                if (kv.Value != null)
                    kv.Value.SetActive(false);
            _bindings.Clear();
        }

        /// <summary>
        /// 每帧 MirrorPool.Sync 后调：读 blob world matrix 同步外部 GO 的 TRS + visible + sortingOrder。
        /// blob 中不存在的绑定节点 → SetActive(false)（node 消失）。
        /// </summary>
        public void Sync(FrameBlob blob)
        {
            if (!blob.IsValid) return;
            _seenThisFrame.Clear();

            // 扫描 blob，同步已绑定的节点
            for (int i = 0; i < blob.NodeCount; i++)
            {
                uint id = blob.NodeId(i);
                if (!_bindings.TryGetValue(id, out var go) || go == null) continue;

                _seenThisFrame.Add(id);

                // TRS 分解（剪切 case 降级，对粒子/3D 模型足够）
                float a = blob.Ma(i), b = blob.Mb(i), c = blob.Mc(i), d = blob.Md(i);
                float rot = Mathf.Atan2(b, a) * Mathf.Rad2Deg;
                float sx = Mathf.Sqrt(a * a + b * b);
                float sy = Mathf.Sqrt(c * c + d * d);
                go.transform.localPosition = new Vector3(blob.Mtx(i), blob.Mty(i), 0);
                go.transform.localRotation = Quaternion.Euler(0, 0, rot);
                go.transform.localScale = new Vector3(sx, sy, 1);

                // sort_key → Renderer.sortingOrder（照 fgui GoWrapper，多 renderer 保相对序）
                foreach (var r in go.GetComponentsInChildren<Renderer>())
                {
                    if (r != null) r.sortingOrder = (int)blob.SortKey(i);
                }

                if (!go.activeSelf) go.SetActive(true);
            }

            // blob 中不存在的绑定节点 → SetActive(false)（node 消失）
            foreach (var kv in _bindings)
            {
                if (!_seenThisFrame.Contains(kv.Key) && kv.Value != null)
                    kv.Value.SetActive(false);
            }
        }
    }
}
