using System.Collections.Generic;
using UnityEngine;

namespace LoomGUI
{
    /// <summary>
    /// NativeHost：外部 GO 跟随 UI 节点 transform + 显隐 + 排序。两层结构：
    ///   - per-node wrapper GO：Sync 每帧设其 transform 跟随 UI 节点 world。
    ///   - 用户 GO 挂 wrapper 下，自身 transform（含 scale 放大）完全用户控制，不被 Sync 覆盖。
    ///
    /// 渲染顺序：
    ///   - GO + wrapper layer = LoomUILayer（UI 相机渲染）
    ///   - GO material renderQueue=3000（Transparent，跟 UI 同队列）
    ///   - GO sortingOrder = 节点 sort_key（UI 列表顺序）
///
    /// LoomGUI root localScale=(sf,-sf,sf) 在 transform 做 y-flip。
    /// GO 直挂 root → handedness flip → 3D mesh winding 反 → 被 Cull Back 剔除。
    /// 解法：_container 挂 root、localScale=(1,-1,1)，worldScale=(sf,sf,sf) positive → 子树 handedness 正常。
    /// </summary>
    internal sealed class NativeHostManager
    {
        private readonly Dictionary<uint, GameObject> _bindings = new();   // node_id → 用户 GO
        private readonly Dictionary<uint, GameObject> _wrappers = new();   // node_id → wrapper GO（跟随 UI）
        private Transform _root;
        private GameObject _container;  // 挂 root、localScale (1,-1,1) 翻正 handedness
        private readonly HashSet<uint> _seenThisFrame = new();

        public void Init(Transform root)
        {
            _root = root;
            // _container：挂 root（继承 design→world position），localScale (1,-1,1) 抵消 root y-flip。
            // container.worldScale = root.scale × (1,-1,1) = (sf,sf,sf) positive → 子树 handedness 正常。
            _container = new GameObject("LoomNativeHost") { hideFlags = HideFlags.DontSaveInEditor };
            _container.transform.SetParent(root, false);
            _container.transform.localScale = new Vector3(1, -1, 1);
            _container.transform.localRotation = Quaternion.identity;
            _container.transform.localPosition = Vector3.zero;
            _container.layer = root.gameObject.layer;  // LoomUILayer
        }

        public void Bind(uint nodeId, GameObject go)
        {
            if (go == null) return;
            Unbind(nodeId);
            // per-node wrapper：Sync 设其 transform 跟随 UI 节点。
            var wrapper = new GameObject("LoomNH_" + nodeId) { hideFlags = HideFlags.DontSaveInEditor };
            wrapper.transform.SetParent(_container.transform, false);
            wrapper.layer = _root.gameObject.layer;
            // 用户 GO 挂 wrapper。
            go.transform.SetParent(wrapper.transform, false);
            SetLayerRecursive(go, _root.gameObject.layer);
            CacheRenderers(go);
            _bindings[nodeId] = go;
            _wrappers[nodeId] = wrapper;
        }

        static void SetLayerRecursive(GameObject go, int layer)
        {
            go.layer = layer;
            foreach (Transform t in go.GetComponentsInChildren<Transform>(true))
                t.gameObject.layer = layer;
        }

        /// MeshRenderer/SkinnedMeshRenderer material renderQueue=3000
        /// （Transparent，跟 UI 同队列，sortingOrder 跨 UI/GO 统一排序）。改 sharedMaterial（非 clone）。
        static void CacheRenderers(GameObject go)
        {
            foreach (var r in go.GetComponentsInChildren<Renderer>(true))
            {
                if (r == null) continue;
                if (r is MeshRenderer || r is SkinnedMeshRenderer)
                {
                    foreach (var mat in r.sharedMaterials)
                    {
                        if (mat != null && mat.renderQueue != 3000) mat.renderQueue = 3000;
                    }
                }
            }
        }

        public void Unbind(uint nodeId)
        {
            if (_bindings.TryGetValue(nodeId, out var go))
            {
                go.SetActive(false);
                _bindings.Remove(nodeId);
            }
            if (_wrappers.TryGetValue(nodeId, out var wrapper))
            {
                if (Application.isPlaying) Object.Destroy(wrapper);
                else Object.DestroyImmediate(wrapper);
                _wrappers.Remove(nodeId);
            }
        }

        public void Clear()
        {
            foreach (var kv in _bindings)
                if (kv.Value != null) kv.Value.SetActive(false);
            _bindings.Clear();
            foreach (var kv in _wrappers)
            {
                if (kv.Value != null)
                {
                    if (Application.isPlaying) Object.Destroy(kv.Value);
                    else Object.DestroyImmediate(kv.Value);
                }
            }
            _wrappers.Clear();
        }

        /// <summary>
        /// 每帧 MirrorPool.Sync 后调：同步 wrapper transform（跟随 UI 节点）+ GO sortingOrder + visible。
        /// 用户 GO 自身 transform 不动（scale 放大等保留）。blob 中不存在 → SetActive(false)。
        /// </summary>
        public void Sync(FrameBlob blob)
        {
            if (!blob.IsValid) return;
            _seenThisFrame.Clear();
            float sf = Mathf.Abs(_root.localScale.y);  // root (sf,-sf,sf) → 取 |y|

            for (int i = 0; i < blob.NodeCount; i++)
            {
                uint id = blob.NodeId(i);
                if (!_wrappers.TryGetValue(id, out var wrapper) || wrapper == null) continue;

                _seenThisFrame.Add(id);

                // TRS 分解（剪切 case 降级）
                float a = blob.Ma(i), b = blob.Mb(i), c = blob.Mc(i), d = blob.Md(i);
                float rot = Mathf.Atan2(b, a) * Mathf.Rad2Deg;
                float sx = Mathf.Sqrt(a * a + b * b);
                float sy = Mathf.Sqrt(c * c + d * d);
                // wrapper 挂 _container（localScale (1,-1,1)）。container.worldScale=(sf,sf,sf)。
                // wrapper.localPosition (Mtx, -Mty, 0)：design y-down → local y 翻，container (1,-1,1) 再翻
                //   → world y = rootPos.y - sf·Mty（与 UI mesh worldPos 一致）。
                // wrapper.localScale (sx, sy, 1/sf)：worldScale = (sx·sf, sy·sf, 1)（z 不压扁）。
                wrapper.transform.localPosition = new Vector3(blob.Mtx(i), -blob.Mty(i), 0);
                wrapper.transform.localRotation = Quaternion.Euler(0, 0, rot);
                wrapper.transform.localScale = new Vector3(sx, sy, sf > 0.0001f ? 1.0f / sf : 1.0f);

                // 用户 GO sortingOrder = 节点 sort_key
                if (_bindings.TryGetValue(id, out var go) && go != null)
                {
                    foreach (var r in go.GetComponentsInChildren<Renderer>())
                        if (r != null) r.sortingOrder = (int)blob.SortKey(i);
                    if (!go.activeSelf) go.SetActive(true);
                }
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
