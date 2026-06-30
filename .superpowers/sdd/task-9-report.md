# Task 9 Report: Shader COLOR_FILTER + MaterialManager + MirrorPool MPB

**Status**: DONE
**Commit**: `1242abc` (main)

## Files Changed

| File | Change | Lines |
|------|--------|-------|
| `loomgui_unity/Assets/LoomGUI/Shaders/LoomGUI-Unlit.shader` | +multi_compile + 5 Properties + 5 CBUFFER + frag branch | +18 |
| `loomgui_unity/Assets/LoomGUI/Runtime/MaterialManager.cs` | +program==3 COLOR_FILTER keyword | +1 |
| `loomgui_unity/Assets/LoomGUI/Runtime/MirrorPool.cs` | +SetColorFilterMatrix helper + program==3 call | +20 |

## Implementation Summary

### LoomGUI-Unlit.shader (4 insertions)

1. **multi_compile** (line 43): `#pragma multi_compile _ COLOR_FILTER` after `BG_COMPOSITE`
2. **Properties** (lines 21-25): `_CF0..3` + `_CFOff` 5 Vector properties after `_ObjM3`, defaulting to identity matrix
3. **CBUFFER** (lines 58-62): 5 `float4` declarations in `UnityPerMaterial` after `_ObjM3`
4. **frag branch** (lines 109-115): `#if defined(COLOR_FILTER)` placed after all color-producing branches (ALPHA_MASK/BG_COMPOSITE/default) and before `#ifdef CLIPPED`. Computes `float4x4 cfM = float4x4(_CF0, _CF1, _CF2, _CF3)` then `col.rgb = mul(cfM, float4(col.rgb, 1.0)).rgb + _CFOff.rgb`. Alpha unchanged (fgui alpha row default identity).

### MaterialManager.cs (1 insertion)

- Line 39: `if (program == 3) mat.EnableKeyword("COLOR_FILTER");` after the existing `BG_COMPOSITE` line.

### MirrorPool.cs (2 insertions)

- Lines 140-143: `if (blob.Program(i) == 3) { SetColorFilterMatrix(ro, blob.ColorMatrix(i)); }` in kind==1 Mesh path, after `SetObjectMatrix` (if non-pure), before `ro.Mr.sharedMaterial = mat`.
- Lines 247-258: `SetColorFilterMatrix` static helper -- uses `ro.Mpb ??=` (shares MPB with `SetObjectMatrix`), `SetVector` x5 for `_CF0..3` + `_CFOff`, then `ro.Mr.SetPropertyBlock(ro.Mpb)`. Indexing: row r = m[r*5..r*5+3], offset[r] = m[r*5+4] (matches fgui UpdateMatrix layout).

## Grep Verification

**Command 1:** `grep -rn "COLOR_FILTER\|_CF0\|_CFOff\|SetColorFilterMatrix" loomgui_unity/Assets/LoomGUI/`

Hit count: 16 (shader 10, MaterialManager 1, MirrorPool 5). Well above >=7 threshold. FrameBlob.cs also references ColorMatrix but that is Task 5's work.

Key hits:
```
Shaders/LoomGUI-Unlit.shader:43:   #pragma multi_compile _ COLOR_FILTER
Shaders/LoomGUI-Unlit.shader:21:   _CF0 ("CF0", Vector) = (1,0,0,0)
Shaders/LoomGUI-Unlit.shader:25:   _CFOff ("CFOff", Vector) = (0,0,0,0)
Shaders/LoomGUI-Unlit.shader:58:   float4 _CF0;
Shaders/LoomGUI-Unlit.shader:62:   float4 _CFOff;
Shaders/LoomGUI-Unlit.shader:109:  #if defined(COLOR_FILTER)
Shaders/LoomGUI-Unlit.shader:112:  float4x4 cfM = float4x4(_CF0, _CF1, _CF2, _CF3);
Shaders/LoomGUI-Unlit.shader:113:  col.rgb = mul(cfM, float4(col.rgb, 1.0)).rgb + _CFOff.rgb;
MaterialManager.cs:39:             if (program == 3) mat.EnableKeyword("COLOR_FILTER");
MirrorPool.cs:142:                 SetColorFilterMatrix(ro, blob.ColorMatrix(i));
MirrorPool.cs:249:                 static void SetColorFilterMatrix(RenderObj ro, float[] m)
MirrorPool.cs:252:                 ro.Mpb.SetVector("_CF0", ...);
MirrorPool.cs:256:                 ro.Mpb.SetVector("_CFOff", ...);
```

**Command 2:** `grep -n "blob.Program(i) == 3\|SetColorFilterMatrix" MirrorPool.cs`

```
140:                    if (blob.Program(i) == 3)
142:                        SetColorFilterMatrix(ro, blob.ColorMatrix(i));
249:        static void SetColorFilterMatrix(RenderObj ro, float[] m)
```

Program==3 MPB call + helper confirmed.

## Self-Review

1. **Completeness**: All required changes present -- shader (multi_compile + 5 Properties + 5 CBUFFER + frag branch), MaterialManager keyword, MirrorPool (MPB helper + call).
2. **Quality**: COLOR_FILTER correctly placed after color branches before CLIPPED (post-processing, allows stacking on program=2). MPB shared with SetObjectMatrix via `ro.Mpb ??=`. Matrix indices match fgui UpdateMatrix layout. Default _CF properties are identity matrix (alpha passthrough).
3. **Discipline**: No unrelated changes. No Text ColorFilter (Text is program=1, ColorFilter only on program=3). YAGNI clean.
4. **Verification**: Both grep commands pass. COLOR_FILTER appears in all expected locations (shader 4 places, MaterialManager 1, MirrorPool 2+).

## Concerns

- None. No Unity compiler on this machine -- PlayMode validation on home machine. Static grep confirms all required code is in place. Shader math, MPB indices, and keyword patterns match existing conventions (BG_COMPOSITE/OBJECT_MATRIX patterns from pitfall 79).
