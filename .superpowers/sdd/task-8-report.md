# v1d.4 家里机 PlayMode 验收清单

## sample：LoomTweenDemo（挂 LoomStage 同 GO，html 含 id="popup" 节点）

- [ ] fade-in：popup opacity 0→1，0.3s 内平滑（不闪现）
- [ ] pop-in：popup scale 0.8→1.0 BackOut，末段 overshoot 略 >1 再回落（BackOut 特征）
- [ ] 两者并发无冲突（opacity 与 scale 不同通道）
- [ ] onComplete：Console 出 `[LoomTweenDemo] tween complete: tag=1 prop=0`（fade）和 `tag=2 prop=2`（scale）
- [ ] 颜色：手改 demo 加 BgColor tween → 背景色平滑渐变
- [ ] kill：调 KillTween 后动画停、停在末值（不弹回）
- [ ] clear：调 ClearAnim 后节点回 CSS（opacity 1 / scale 1）
- [ ] 零回归：无 tween 的既有场景（v1d.3 sample）渲染/交互不变
- [ ] stress：500 节点场景 + 几个 tween 无卡顿
