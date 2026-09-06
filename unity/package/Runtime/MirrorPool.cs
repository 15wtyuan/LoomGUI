using System.Collections.Generic;
using UnityEngine;

namespace Yio
{
    /// 渲染树 → GameObject 镜像 diff。每帧 O(n)：标 stale → 遍历命中清 stale/更新 → 余销毁。
    /// flatten：所有 GO 挂 root；纯平移节点 localPosition=(Mtx,Mty) 绝对 design；非纯平移节点
    /// GO transform=identity + _ObjectMatrix uniform；sortingOrder=sort_key。
    /// parent_id 仍在 blob 列但渲染不用（事件系统再用）。
    /// Mesh 顶点已由 Rust re-base 到节点本地空间，此处按 (x,y,0) 上传。
    /// change_level 三分支：0=SKIP(保留GO) 1=HEADER(只更header,不重建mesh) 2=FULL(重建mesh)。
    /// v10：所有渲染节点统一走 mesh 路径（text 核心自产 atlas 产 Mesh，kind 只有 1=Mesh）。
    sealed class RenderObj
    {
        public GameObject Go;
        public MeshFilter Mf;
        public MeshRenderer Mr;
        public Mesh Mesh;
        public bool Stale;
        public ulong LastNodeId;      // 诊断：最近绑定的 node_id（DumpState 打印；不做复用校验——复用换绑是 reuse_key 池化的正常行为）

        // #62 页纹理逐出：本 obj 当前绑定的图集页键（null=非页承载：字体页/miss/纯色）。
        // UpdateHeader 每 lean 帧随 look 重写；Skip 帧不重写——但 Skip 行本就「视觉与上帧
        // 相同」，绑定不变是精确语义。Sync 每帧对 active obj 的绑定页盖章续命。
        public (int, int)? BoundPage;

        // #66：mesh 原始（未补偿）AABB。FULL 帧上传后 RecalculateBounds 时存底；剔除补偿
        // 永远基于此值重算。不能读 Mesh.bounds 顶替：Header 帧不重建 mesh，Mesh.bounds 里
        // 是上一帧「已补偿值」，再乘一次线性矩阵得 AABB(L·AABB(L·B)) ≠ AABB(L·B)——滚动中
        // 的旋转/缩放节点每帧叠加（scale<1 几何级缩小、45° 无界膨胀）。缓存底也让 L 本身
        // 变化的帧（transform 动画）从原始 bounds 直算，无叠加残留。
        public Bounds RawMeshBounds;

        // buffer 复用（500 节点静态压测 GC 缓解）：每 RenderObj 持可复用 List，
        // UploadMesh 每帧 Clear+fill 后用 Mesh.SetVertices(List) 等 overload 上传——
        // List<T>.Clear() 保留 Capacity，故 warm-up 后零 per-frame 数组 alloc。
        public readonly List<Vector3> VList = new();
        public readonly List<UnityEngine.Vector2> UvList = new();
        public readonly List<UnityEngine.Color> CList = new();
        public readonly List<int> IList = new();
        // cached MaterialPropertyBlock for per-renderer uniforms (_ObjM, _CF, _Alpha).
        // Lazy-init; consolidated into single SetPropertyBlock per frame.
        public MaterialPropertyBlock Mpb;
    }

    public sealed class MirrorPool
    {
        // 双 dict keying。reuse_key>0 的 slot 节点按 reuse_key 复用 GO
        // （slot 换绑 item 时 NodeId 变但 reuse_key 不变 → GO 不销毁重建）；
        // reuse_key=0 的普通节点按 node_id keying（v1 行为不变）。
        // v14：node_id u64（#26）。_poolByReuse 的 key 是 reuse_key（u32 ordinal），
        // 拓宽到 ulong 统一两 dict 类型（同一 pool 变量交替引用）；两 dict 分立，key 无碰撞面。
        /// <summary>A4 跨 Stage 排序基址（hub 按 stage 层序分配；同相机下各 stage 的
        /// sort_key 都从 0 起编，不偏移会互相穿插）。Driver 每帧经 backend.SetSortBase 下发。</summary>
        internal int SortBase;

        readonly Dictionary<ulong, RenderObj> _poolByNodeId = new();
        readonly Dictionary<ulong, RenderObj> _poolByReuse = new();
        // world-space 挂载容器登记（#109 C8）：槽位 → 业务摆放的 3D 容器 Transform。
        // blob mount_id 列非 0 的行 SetParent 到对应容器（顶点已 re-base 到挂载根局部系，
        // 容器层随业务 → 场景相机渲染 + ZTest LEqual 吃 3D 深度遮挡）。
        readonly Dictionary<ulong, Transform> _mountContainers = new();


        /// <summary>
        /// 行归属解析：挂载行（mount_id ≠ 0 且容器登记存活）→ 容器；其余（含容器已销毁的
        /// 降级兜底）→ 屏幕 root。容器被业务销毁时行回落屏幕路径——可见性降级但不悬空。
        /// </summary>
        Transform ResolveParent(Transform root, ulong mountId)
        {
            if (mountId != 0
                && _mountContainers.TryGetValue(mountId, out var c)
                && c != null)
                return c;
            return root;
        }

        /// <summary>登记挂载容器（Driver BindWorldMount → backend 转发）。</summary>
        internal void SetMountContainer(ulong slot, Transform container) =>
            _mountContainers[slot] = container;

        /// <summary>
        /// 解除挂载容器：先把容器下存活镜像 GO 挂回屏幕 root（core 侧 mount 已清 0，本帧
        /// Sync 的 UpdateHeader 会自然回落 root——这里先行归位防容器销毁连带销毁镜像 GO），
        /// 再移除登记。容器 GO 本体的销毁归 Driver（容器是它建的）。
        /// </summary>
        internal void ClearMountContainer(ulong slot, Transform root)
        {
            if (!_mountContainers.TryGetValue(slot, out var c) || c == null)
            {
                _mountContainers.Remove(slot);
                return;
            }
            _mountContainers.Remove(slot);
            ReparentFromContainer(_poolByNodeId, c, root);
            ReparentFromContainer(_poolByReuse, c, root);
        }

        static void ReparentFromContainer(Dictionary<ulong, RenderObj> pool, Transform container, Transform root)
        {
            foreach (var ro in pool.Values)
            {
                if (ro.Go != null && ro.Go.transform.parent == container)
                    ro.Go.transform.SetParent(root, false);
            }
        }

        /// 当前镜像中的 GO 数量（两 dict 之和）。测试/调试用。
        public int Count => _poolByNodeId.Count + _poolByReuse.Count;

        /// <summary>
        /// 镜像 diff：遍历 blob 节点 → 建/更新/复用 GO。
        /// v10：所有节点统一走 mesh 路径（核心自绘字体产 Mesh，text 不再单分一道）。
        /// </summary>
        public void Sync(FrameBlob blob, Transform root, MaterialManager mm,
                         SpriteResolver sprites, Texture fallback)
        {
            // 防御：陈旧/非当前 blob 直接早退（magic+version 校验）。不做清理——上一帧的 GO
            // 维持不动比误销毁更安全；调用方应自检 IsValid 再 Sync。
            if (!blob.IsValid) return;

            // #62 镜像侧续命：Skip 行不进 lean 段（变更帧零 GetSprite），「闲置页仍被画」
            // 的证据只能从镜像取——active GO = 本帧在屏，其绑定页照章盖章。缺这步，静态页
            // 的图集页在宽限期满被销毁而材质仍引用已销毁纹理（图标蒸发）。池 ≤ 数百量级，
            // 每帧迭代纳秒级；盖章只写缓存命中项，零分配。先盖章收本帧证据，再 Sweep 裁决。
            if (sprites != null)
            {
                if (_poolByNodeId.Count > 0)
                    foreach (var kv in _poolByNodeId)
                        if (kv.Value.BoundPage.HasValue && kv.Value.Go.activeSelf)
                            sprites.StampPage(kv.Value.BoundPage.Value);
                if (_poolByReuse.Count > 0)
                    foreach (var kv in _poolByReuse)
                        if (kv.Value.BoundPage.HasValue && kv.Value.Go.activeSelf)
                            sprites.StampPage(kv.Value.BoundPage.Value);

                // 页纹理逐出心跳（#62）：Sync 是每帧必经口，顺路驱动 SpriteResolver 扫一遍
                // 闲置页。策略细节（宽限/盖章/字体豁免）全封在 SpriteResolver，此处不感知。
                sprites.Sweep();
            }

            // ① 全标 stale（两个 dict）
            foreach (var kv in _poolByNodeId) kv.Value.Stale = true;
            foreach (var kv in _poolByReuse) kv.Value.Stale = true;
            // clip 链数组刷新：每帧为 blob 表里**每个** ctx 全量重设（不依赖 lean 行——
            // idle 全 Skip 帧也刷；ctx 少、数组小，开销可忽略）。SetClipEntries 先写
            // dict：本帧新建的材质（mm.Get）创建时即带上。
            foreach (uint clipCtx in blob.ClipContextIds())
                mm.SetClipEntries(clipCtx, blob.ReadClipEntries(clipCtx));

            // ② v15 skip 段先走：Skip 行 = 「对象还在，清 stale」；parked keepalive =
            //    保留 GO 并隐藏。这行不进 SOA（16B/条）——后端不读 header/mesh。
            //    parked 条目可能是 slot 根（reuse_key>0）或其后代（reuse_key=0 按
            //    node_id 池化）——两者都要保留：park 剪整子树，若只保根，后代 GO 被
            //    stale 销毁，reactivate 重建（每帧滚动 churn）。无先验 GO → no-op。
            for (int s = 0; s < blob.SkipCount; s++)
            {
                bool parked = blob.SkipParked(s);
                uint rk = blob.SkipReuseKey(s);
                ulong poolKey = rk != 0 ? rk : blob.SkipNodeId(s);
                Dictionary<ulong, RenderObj> pool = rk != 0 ? _poolByReuse : _poolByNodeId;
                if (pool.TryGetValue(poolKey, out var roK))
                {
                    roK.Stale = false;
                    if (parked && roK.Go.activeSelf) roK.Go.SetActive(false);
                }
            }

            // ③ lean 段遍历（Header/Full 行；v8 三分支的 Skip 分支已被 skip 段接管）。
            int n = blob.LeanCount;
            for (int i = 0; i < n; i++)
            {
                ulong id = blob.NodeId(i);
                uint reuseKey = blob.ReuseKey(i);    // 虚拟列表
                ulong poolKey = reuseKey != 0 ? reuseKey : id;
                Dictionary<ulong, RenderObj> pool = reuseKey != 0 ? _poolByReuse : _poolByNodeId;

                // visible=0（世界锚点出屏的 render_hidden）：保留镜像对象、隐藏——与
                // display:none 的「剪除条目」正交（那些节点根本不产行，走 stale 销毁路径）。
                if (!blob.Visible(i))
                {
                    if (pool.TryGetValue(poolKey, out var roH))
                    {
                        roH.Stale = false;
                        if (roH.Go.activeSelf) roH.Go.SetActive(false);
                    }
                    continue;
                }
                byte kind = blob.PayloadKind(i);
                byte level = blob.ChangeLevel(i);   // 1=Header 2=Full（Skip 不在此）

                // v10：kind 只有 1=Mesh（文本核心自产 atlas 走同路径）。
                if (kind != 1) continue;

                // 解决图资源（path_idx → path → SpriteResolver.GetSprite）。
                // 文本节点的 font-atlas path（yio://font-atlas/...）由 RegisterFontAtlasPage 注册
                // 进 SpriteResolver，GetSprite 命中返 SpriteLookup——tex 回落 fallback whiteTexture，
                // 由 SyncFontAtlas 换贴真实 atlas。
                SpriteLookup look = default; Texture tex = fallback;
                uint pathIdx = blob.PathIdx(i);
                if (pathIdx != 0 && sprites != null)
                {
                    string path = blob.ReadPath(pathIdx);
                    if (!string.IsNullOrEmpty(path))
                    {
                        look = sprites.GetSprite(path);
                        if (look.found) tex = look.tex;
                    }
                }

                // 确保 RenderObj 存在；新建 GO 无 mesh → 强制 FULL（无视 blob 的 HEADER）
                if (!pool.TryGetValue(poolKey, out var ro))
                {
                    ro = NewRenderObj(ResolveParent(root, blob.MountId(i)));
                    pool[poolKey] = ro;
                    level = 2; // 强制 FULL
                }
                ro.LastNodeId = id; // 新建 + 复用均更新（slot 换绑时 node_id 变）
                ro.Stale = false;
                if (!ro.Go.activeSelf) ro.Go.SetActive(true); // reactivate parked→active

                UpdateHeader(ro, blob, i, root, mm, kind, look, tex);
                if (level == 2)
                {
                    UploadMeshOrText(ro, blob, i, look);
                    ro.RawMeshBounds = ro.Mesh.bounds; // 原始 AABB 存底（补偿前）
                }
                // #66：非纯平移节点的 Mesh.bounds 补偿须在 upload 之后（RecalculateBounds
                // 会覆盖），且从 RawMeshBounds 缓存底重算——Header 帧不重建 mesh，
                // 读 Mesh.bounds 会拿到上帧已补偿值再乘一次 L（非幂等，见 RenderObj 字段注释）。
                if (!blob.IsPureTranslation(i)) CompensateMeshBoundsForLinear(ro, blob, i);
            }

            // ③ 余 stale 销毁（两个 dict）
            var dead1 = new List<ulong>();
            foreach (var kv in _poolByNodeId) if (kv.Value.Stale) dead1.Add(kv.Key);
            foreach (var id in dead1) { TearDown(_poolByNodeId[id]); _poolByNodeId.Remove(id); }
            var dead2 = new List<ulong>();
            foreach (var kv in _poolByReuse) if (kv.Value.Stale) dead2.Add(kv.Key);
            foreach (var id in dead2) { TearDown(_poolByReuse[id]); _poolByReuse.Remove(id); }
        }

        /// 更新 GO header（position/rotation/scale + sortingOrder + clip + material + per-renderer uniforms）。
        /// 无论 HEADER 还是 FULL 路径均调用；仅 SKIP 跳过。
        /// v10：不再有 kind==2 text 单独材质路径——所有节点统一走 program+tex 选材。
        void UpdateHeader(RenderObj ro, FrameBlob blob, int i, Transform root,
                          MaterialManager mm, byte kind, SpriteLookup look, Texture tex)
        {
            // #62：绑定页键随 look 刷新（null=字体页/miss/纯色，不参与续命）。Skip 帧
            // 不进本函数——绑定沿用上帧，与「Skip=视觉未变」的行语义一致。
            ro.BoundPage = look.PageKey;
            // flatten：所有节点挂 root（挂载行除外——路由到业务 3D 容器，见 ResolveParent）。
            // pure 和非 pure 统一 GO localPosition=(Mtx,Mty)（world translate 进 GO transform）。
            // 非纯平移的 scale/rotate 进 _ObjectMatrix（无 translate）。translate 进 GO
            // localPosition；但 renderer.bounds = GO 平移 × 未旋转 mesh ≠ 旋转后真实
            // AABB——剔除补偿见 CompensateMeshBoundsForLinear（#66）。
            Transform parent = ResolveParent(root, blob.MountId(i));
            ro.Go.transform.SetParent(parent, false);
            if (ro.Go.layer != parent.gameObject.layer)
                ro.Go.layer = parent.gameObject.layer; // 挂载行随容器层（场景层 → 3D 深度遮挡）
            bool pure = blob.IsPureTranslation(i);
            ro.Go.transform.localPosition = new Vector3(blob.Mtx(i), blob.Mty(i), 0f);
            ro.Go.transform.localRotation = Quaternion.identity;
            ro.Go.transform.localScale = Vector3.one;

            ro.Mr.sortingOrder = (int)blob.SortKey(i) + SortBase;

            uint maskCtx = blob.MaskContext(i);

            // 材质：按 program+texture 选（包括 text 节点，program=1，tex 由 SyncFontAtlas 注入 font atlas）。
            Material mat = mm.Get((int)blob.Program(i), tex, maskCtx, !pure);
            if (mat != null) ro.Mr.sharedMaterial = mat;

            // 合并 per-renderer uniform（MPB 一次 SetPropertyBlock，避免 _ObjM/_CF/_Alpha 互相覆盖）。
            // _ObjM：非纯平移时传 scale/rotate 矩阵（纯平移 = shader 默认 identity，不设）。
            // _CF：ColorFilter（program 3/4）传 5 Vector；其他不设。
            // _Alpha：每帧无条件设（alpha 剥离顶点色）。
            float alpha = blob.Alpha(i);
            bool hasFilter = blob.Program(i) == 3 || blob.Program(i) == 4;

            ro.Mpb ??= new MaterialPropertyBlock();
            ro.Mr.GetPropertyBlock(ro.Mpb);
            if (!pure)
            {
                // _ObjectMatrix 只 scale/rotate（translate 进 GO localPosition，renderer.bounds 自动 world）。
                var objM = Matrix4x4.identity;
                objM[0, 0] = blob.Ma(i); objM[0, 1] = blob.Mc(i);
                objM[1, 0] = blob.Mb(i); objM[1, 1] = blob.Md(i);
                ro.Mpb.SetVector("_ObjM0", objM.GetRow(0));
                ro.Mpb.SetVector("_ObjM1", objM.GetRow(1));
                ro.Mpb.SetVector("_ObjM2", objM.GetRow(2));
                ro.Mpb.SetVector("_ObjM3", objM.GetRow(3));
            }
            // ColorFilter（program=3=filter 无图 / 4=filter+bg-image 双 keyword）：
            // 矩阵 20 float 拆 5 Vector MPB SetVector。
            if (hasFilter)
            {
                float[] cf = blob.ColorMatrix(i);
                ro.Mpb.SetVector("_CF0", new Vector4(cf[0],  cf[1],  cf[2],  cf[3]));
                ro.Mpb.SetVector("_CF1", new Vector4(cf[5],  cf[6],  cf[7],  cf[8]));
                ro.Mpb.SetVector("_CF2", new Vector4(cf[10], cf[11], cf[12], cf[13]));
                ro.Mpb.SetVector("_CF3", new Vector4(cf[15], cf[16], cf[17], cf[18]));
                ro.Mpb.SetVector("_CFOff", new Vector4(cf[4], cf[9], cf[14], cf[19]));
            }
            // SDF 文字效果（program=1 ALPHA_MASK）：读 effect_block 列 → per-renderer MPB。
            // 非 text 节点不设（material 默认全 0 = 纯 face）。effect 参数变化经
            // header_hash 走 ChangeLevel::Header（不重建 mesh，仅刷新 MPB）。
            // _UnderlayOffset 是 float4（shader 取 .xy 做像素偏移，.zw 兜 0）。
            if (blob.Program(i) == 1)
            {
                float[] eb = blob.EffectBlock(i);
                ro.Mpb.SetFloat("_OutlineWidth", eb[0]);
                ro.Mpb.SetVector("_OutlineColor", new Vector4(eb[1], eb[2], eb[3], eb[4]));
                for (int s = 0; s < 3; s++)
                {
                    int b = 5 + s * 7; // underlay 槽起点：[5]/[12]/[19]
                    ro.Mpb.SetVector("_UnderlayOffset" + s, new Vector4(eb[b], eb[b + 1], 0, 0));
                    ro.Mpb.SetFloat("_UnderlaySoftness" + s, eb[b + 2]);
                    ro.Mpb.SetVector("_UnderlayColor" + s, new Vector4(eb[b + 3], eb[b + 4], eb[b + 5], eb[b + 6]));
                }
                ro.Mpb.SetFloat("_GlowPower", eb[26]);
                ro.Mpb.SetVector("_GlowColor", new Vector4(eb[27], eb[28], eb[29], eb[30]));
                ro.Mpb.SetFloat("_BlurWidth", eb[31]);
            }
            // box-shadow blur（program=5 SHADOW_BLUR）：读 shadow_params 列 → per-renderer MPB。
            // shadow_params = [halfSize.xy, radius, σ, inset, _pad]。非 shadow 节点全零，
            // 故仅 program==5 时设（避免给非 shadow 节点误设全零 uniform）。参数变化经
            // header_hash 走 ChangeLevel::Header（不重建 mesh，仅刷新 MPB，照 effect 同路径）。
            if (blob.Program(i) == 5)
            {
                float[] sp = blob.ShadowParams(i);
                ro.Mpb.SetVector("_ShadowHalfSize", new Vector4(sp[0], sp[1], 0, 0));
                ro.Mpb.SetFloat("_ShadowRadius", sp[2]);
                ro.Mpb.SetFloat("_ShadowSigma", sp[3]);
                ro.Mpb.SetFloat("_ShadowInset", sp[4]);
            }
            // 背景渐变（program=6 GRADIENT / 7 GRADIENT+COLOR_FILTER）：读 grad_params 列 →
            // per-renderer MPB。stops 拆 8 组 Vector4(rgba) + Float(pos)——MPB 只覆盖 Properties
            // 声明过的属性（_ObjectMatrix 踩坑同源），ShaderLab 不支持数组属性，故逐槽
            // SetVector/SetFloat（照 _Underlay* 编号属性先例）。未用槽填「末 stop 色 @pos=1」
            // ——shader 无需 count uniform，8 槽段搜索自然退化到末 stop。
            if (blob.Program(i) == 6 || blob.Program(i) == 7)
            {
                float[] gp = blob.GradParams(i);
                int n = Mathf.Min((int)gp[10], 8);
                if (n < 1) n = 1;
                ro.Mpb.SetFloat("_GradKind", gp[0]);
                ro.Mpb.SetVector("_GradGeom", new Vector4(gp[2], gp[3], gp[4], gp[5]));
                ro.Mpb.SetVector("_GradGeom2", new Vector4(gp[6], gp[7], gp[8], gp[9]));
                // 末 stop（未用槽的填充源；radial 远端 / linear clamp 端点色）。
                int last = 12 + (n - 1) * 5;
                Vector4 lastCol = new Vector4(gp[last], gp[last + 1], gp[last + 2], gp[last + 3]);
                for (int s = 0; s < 8; s++)
                {
                    int b = 12 + s * 5;
                    bool used = s < n;
                    ro.Mpb.SetVector("_GradStop" + s, used
                        ? new Vector4(gp[b], gp[b + 1], gp[b + 2], gp[b + 3])
                        : lastCol);
                    ro.Mpb.SetFloat("_GradPos" + s, used ? gp[b + 4] : 1f);
                }
            }
            ro.Mpb.SetFloat("_Alpha", alpha);
            // _ObjT：节点 design 平移（Mtx,Mty）——clip 链测试空间是 design 坐标，
            // blob 顶点已 re-base 到本地（见 _ObjT 注释），shader 侧补回。
            ro.Mpb.SetVector("_ObjT", new Vector4(blob.Mtx(i), blob.Mty(i), 0f, 0f));
            ro.Mr.SetPropertyBlock(ro.Mpb);
        }

        /// 上传 mesh / 重建 mesh（仅 FULL 路径调用）。
        /// v10：不再有 kind==2 text 分支——所有节点统一走 mesh 上传（核心自产 atlas，word mesh 已含字形 UV）。
        static void UploadMeshOrText(RenderObj ro, FrameBlob blob, int i,
                                     SpriteLookup look)
        {
            // mesh 上传（顶点已 re-base 到本地）。
            var seg = blob.ReadMesh(i);
            UploadMesh(ro, seg);
            ro.Mesh.RecalculateBounds();
            // path → SpriteResolver.GetSprite → look。look.found=false（path_idx=0 纯色 / 查不到）则跳过重映射，
            // mesh 沿用 blob 全图 UV [0,1] + fallback whiteTexture。
            // look.found → RemapMeshUvToSprite 把全图 UV 重映射到 sprite 在 atlas 的子区（用 look.uvRect）。
            if (look.found)
                RemapMeshUvToSprite(ro, look.uvRect);
        }

        /// <summary>
        /// #66：非纯平移节点的剔除 bounds 补偿。rotate/scale 走 _ObjM shader 矩阵、GO 只带
        /// 平移分量 → Unity renderer.bounds = GO 平移 × 未旋转 mesh，与真实视觉 AABB（线性矩阵
        /// 旋转后的四边形）中心错位且范围偏小——旋转条横放假 bounds 竖直方向仅 h px，旋转 45°
        /// 后真实竖直 ≈ w·sinθ → 滚动中真身进视口而假 bounds 在外，被 SRP 错误剔除（滚动容器
        /// 内旋转连线消失）。补偿：bounds 置「线性矩阵 × 顶点 AABB」的 AABB（仍在 GO 本地系，
        /// 随 GO 平移）→ 剔除结果 = T × AABB(L·verts) = 真实世界 AABB。z 向原样透传（2D）。
        /// </summary>
        static void CompensateMeshBoundsForLinear(RenderObj ro, FrameBlob blob, int i)
        {
            // 从缓存的原始 AABB 算（非 Mesh.bounds——那里可能是上帧已补偿值，再乘会叠加）。
            Bounds b = ro.RawMeshBounds;
            Vector3 c = b.center;
            Vector3 e = b.extents;
            float ma = blob.Ma(i), mb = blob.Mb(i), mc = blob.Mc(i), md = blob.Md(i);
            float minX = float.MaxValue, minY = float.MaxValue, maxX = float.MinValue, maxY = float.MinValue;
            for (int sx = -1; sx <= 1; sx += 2)
                for (int sy = -1; sy <= 1; sy += 2)
                {
                    float x = c.x + sx * e.x, y = c.y + sy * e.y;
                    float rx = ma * x + mc * y;
                    float ry = mb * x + md * y;
                    if (rx < minX) minX = rx;
                    if (rx > maxX) maxX = rx;
                    if (ry < minY) minY = ry;
                    if (ry > maxY) maxY = ry;
                }
            ro.Mesh.bounds = new Bounds(
                new Vector3((minX + maxX) * 0.5f, (minY + maxY) * 0.5f, c.z),
                new Vector3(maxX - minX, maxY - minY, e.z * 2f));
        }

        static RenderObj NewRenderObj(Transform root)
        {
            var go = new GameObject("yio_node");
            // ExecuteAlways 下镜像 GO 是运行时派生产物，标 DontSaveInEditor 防被存进场景
            // （否则 EditMode Sync 产出的 GO 会 dirty 场景、Play/Stop 与 domain reload 累积残留）。
            go.hideFlags = HideFlags.DontSaveInEditor;
            go.transform.SetParent(root, false);
            go.layer = root.gameObject.layer;  // YioUI
            var mf = go.AddComponent<MeshFilter>();
            var mr = go.AddComponent<MeshRenderer>();
            var mesh = new Mesh { indexFormat = UnityEngine.Rendering.IndexFormat.UInt32 };
            mesh.hideFlags = HideFlags.DontSaveInEditor;  // Mesh 是独立 Object，也别存盘
            mesh.MarkDynamic();
            mf.sharedMesh = mesh;
            return new RenderObj { Go = go, Mf = mf, Mr = mr, Mesh = mesh };
        }

        /// buffer 复用：从 MeshSegment 填 ro 持有的可复用 List，再走 SetVertices(List) 等 overload。
        /// List<T>.Clear() 保留 Capacity → warm-up 后每帧零数组 alloc。
        /// 注意：SetVertices(List) 要求 list 长度 == 顶点数；Clear()+Add 精确填到 Verts.Length 即满足。
        static void UploadMesh(RenderObj ro, MeshSegment seg)
        {
            int vc = seg.Verts.Length;
            // Clear 保留 capacity，再填（避免每帧 new List / new 数组）。
            var v = ro.VList; v.Clear();
            var uv = ro.UvList; uv.Clear();
            var c = ro.CList; c.Clear();
            var idx = ro.IList; idx.Clear();
            // 预扩一次（首次或更大 mesh 时）；后续 Clear 不收缩，零 alloc。
            if (v.Capacity < vc) { v.Capacity = vc; uv.Capacity = vc; c.Capacity = vc; }
            int ic = seg.Idx.Length;
            if (idx.Capacity < ic) idx.Capacity = ic;

            for (int i = 0; i < vc; i++)
            {
                v.Add(new Vector3(seg.Verts[i].x, seg.Verts[i].y, 0f));
                uv.Add(seg.Uvs[i]);
                c.Add(seg.Colors[i]);
            }
            for (int i = 0; i < ic; i++) idx.Add((int)seg.Idx[i]);

            ro.Mesh.Clear();                 // Unity 要求 SetVertices 前清空，否则顶点数变更报错
            ro.Mesh.SetVertices(v);
            ro.Mesh.SetUVs(0, uv);
            ro.Mesh.SetColors(c);
            ro.Mesh.SetTriangles(idx, 0);
        }

        /// 把 mesh UV（core 产 [0,1] 全图 UV）线性映射到 sprite 在 atlas 页内的子区。
        /// uvRect 是打包器算好的 atlas 子区——Unity Rect (x=u0, y=v0, w=u1-u0, h=v1-v0)。
        /// 不内缩半纹素：padding 下 bilinear 边缘 fringe 几乎不可见；开 atlas enableAlphaDilation（边缘像素复制进 padding）防 bleed。
        static void RemapMeshUvToSprite(RenderObj ro, UnityEngine.Rect uvRect)
        {
            // blob UV v 已翻转（TL.v=1），线性映射进子区保持翻转。
            // 直接改 ro.UvList（UploadMesh 已填），不复 alloc 新 List。
            var uvs = ro.UvList;
            for (int i = 0; i < uvs.Count; i++)
                uvs[i] = new UnityEngine.Vector2(
                    uvRect.x + uvs[i].x * uvRect.width,
                    uvRect.y + uvs[i].y * uvRect.height);
            ro.Mesh.SetUVs(0, uvs);
        }

        public void Clear()
        {
            foreach (var kv in _poolByNodeId) TearDown(kv.Value);
            _poolByNodeId.Clear();
            foreach (var kv in _poolByReuse) TearDown(kv.Value);
            _poolByReuse.Clear();
        }

        /// <summary>
        /// 诊断：dump 当前 MirrorPool 里所有活 GO 的状态（node_id / localPosition / mesh bounds）
        /// 到字符串。按 F8 调用，在「好」「坏」两种布局各 dump 一次对比，定位 Unity 侧状态泄漏。
        /// </summary>
        public string DumpState()
        {
            var sb = new System.Text.StringBuilder();
            sb.AppendLine($"[MirrorPool] byNodeId={_poolByNodeId.Count} byReuse={_poolByReuse.Count}");
            sb.AppendLine("  poolKey  nodeId    sort  pos(x,y)            meshBounds(center,size)");
            DumpDict(sb, "id", _poolByNodeId);
            DumpDict(sb, "rk", _poolByReuse);
            return sb.ToString();
        }

        void DumpDict(System.Text.StringBuilder sb, string tag, Dictionary<ulong, RenderObj> pool)
        {
            foreach (var kv in pool)
            {
                var ro = kv.Value;
                if (ro?.Go == null) { sb.AppendLine($"  [{tag}{kv.Key}] (null GO)"); continue; }
                var p = ro.Go.transform.localPosition;
                int sort = ro.Mr != null ? ro.Mr.sortingOrder : 0;
                var b = ro.Mesh != null ? ro.Mesh.bounds : new Bounds();
                // 材质 clip 诊断：CLIPPED keyword + 链 entry 数 + 每 entry kind 回读
                // （材质实际值——shapeKind 1=circle 2=polygon；与 [clip table] 段对拍，
                // 不一致 = 刷新链路断了，一致但视觉旧 = 渲染侧没吃到）。
                string matInfo = "";
                if (ro.Mr != null && ro.Mr.sharedMaterial != null)
                {
                    var m = ro.Mr.sharedMaterial;
                    bool clipped = m.IsKeywordEnabled("CLIPPED");
                    int clipCount = Mathf.RoundToInt(m.GetFloat("_ClipCount"));
                    string kinds = "";
                    if (clipped && clipCount > 0)
                    {
                        var f0 = m.GetVectorArray("_ClipFrame0");
                        var f1 = m.GetVectorArray("_ClipFrame1");
                        for (int e = 0; e < clipCount && e < 4; e++)
                            kinds += (int)f1[e].w == 0 ? "-" : ((int)f1[e].w == 2 ? "R" : "r");
                        kinds += "/";
                        for (int e = 0; e < clipCount && e < 4; e++)
                            kinds += (int)f0[e].w == 0 ? "-" : ((int)f0[e].w == 1 ? "c" : "p");
                    }
                    matInfo = $" clip={clipped} entries={clipCount} kinds=[{kinds}]";
                }
                sb.AppendLine($"  [{tag}{kv.Key}] nid={ro.LastNodeId} sort={sort} pos=({p.x:F0},{p.y:F0}) mb=(({b.center.x:F0},{b.center.y:F0}),({b.size.x:F0},{b.size.y:F0})) active={ro.Go.activeSelf}{matInfo}");
            }
        }

        // Edit-mode-safe 销毁：Driver 可能挂 [ExecuteAlways]，Sync/Clear 会在 Edit mode 跑；
        // Object.Destroy 在 Edit mode 非法（须 DestroyImmediate）。
        static void TearDown(RenderObj ro)
        {
            DestroyObj(ro.Mesh);   // new Mesh() 是独立 UnityEngine.Object，须显式销毁，否则泄漏
            DestroyObj(ro.Go);
        }

        static void DestroyObj(Object o)
        {
            if (o == null) return;
            if (Application.isPlaying) Object.Destroy(o);
            else Object.DestroyImmediate(o);
        }
    }
}
