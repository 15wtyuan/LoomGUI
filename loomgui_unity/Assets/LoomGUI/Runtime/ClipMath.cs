using UnityEngine;

namespace LoomGUI
{
    /// _ClipBox 推导（§4.4，照搬 fgui UpdateContext.cs:105-156）。
    ///
    /// 给 design-space clip rect（绝对，y-down）+ 根 Stage transform：把两角经
    /// root.TransformPoint 转到 world（root scale=(sf,-sf,sf) y-down→y-up），取 world
    /// center/half，按 fgui 公式 `_ClipBox = (-cx/hw, -cy/hh, 1/hw, 1/hh)` 算。
    /// 半宽/高为 0（嵌套 disjoint→空集）→ safe-blank (-2,-2,0,0)：clipPos 恒 (-2,-2)，
    /// max(abs)=2>1 → step(2,1)=0 → 全 discard（防除零）。
    ///
    /// shader 端（LoomGUI-Unlit.shader CLIPPED variant）：
    ///   clipPos = TransformObjectToWorld(pos).xy * _ClipBox.zw + _ClipBox.xy
    ///   col.a *= step(max(abs(clipPos)), 1)
    /// 代入：clipPos = (worldPos.x/hw - cx/hw, worldPos.y/hh - cy/hh) = (worldPos - center)/half。
    /// 区域内 → |clipPos|<=1（保留），外 → >1（discard）。
    public static class ClipMath
    {
        /// fgui safe-blank（UpdateContext.cs:124）：half=0 时返回，clipPos 恒在外 → 全裁。
        public static readonly Vector4 SafeBlank = new Vector4(-2f, -2f, 0f, 0f);

        /// 由 design rect + 根 transform 算 _ClipBox（world 空间 center/half）。
        /// designX/Y/W/H 是绝对 design 坐标（layout 已算绝对，§4.2）。
        public static Vector4 ComputeClipBox(Transform root,
            float designX, float designY, float designW, float designH)
        {
            // 两角 design → world。root.TransformPoint 统一处理 scale(1,-1,1)+pos。
            Vector3 wTL = root.TransformPoint(new Vector3(designX, designY, 0f));
            Vector3 wBR = root.TransformPoint(new Vector3(designX + designW, designY + designH, 0f));

            float cx = (wTL.x + wBR.x) * 0.5f;
            float cy = (wTL.y + wBR.y) * 0.5f;
            float hw = Mathf.Abs(wBR.x - wTL.x) * 0.5f;
            float hh = Mathf.Abs(wBR.y - wTL.y) * 0.5f;

            if (hw == 0f || hh == 0f) return SafeBlank;
            return new Vector4(-cx / hw, -cy / hh, 1f / hw, 1f / hh);
        }
    }
}
