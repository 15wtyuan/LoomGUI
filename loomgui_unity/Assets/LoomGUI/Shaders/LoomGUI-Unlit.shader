Shader "LoomGUI/Unlit"
{
    Properties
    {
        _MainTex ("Texture", 2D) = "white" {}
        _SrcFactor ("SrcFactor", Float) = 5   // SrcAlpha
        _DstFactor ("DstFactor", Float) = 10  // OneMinusSrcAlpha
        _ClipBox ("ClipBox", Vector) = (0,0,1,1)
        // _ObjectMatrix 拆 4 个 Vector 进 Properties（ShaderLab 无 Matrix property 类型）。
        // _ObjectMatrix 声明在 CBUFFER(UnityPerMaterial) 但**无 Properties 对应** → MPB.SetMatrix
        // 不覆盖非 material property 的 CBUFFER 字段 → 非 pure 节点（transform:scale/rotate）
        // _ObjectMatrix 恒默认 → 顶点塌缩到 design 原点（字消失/跑到屏幕左上）。
        // 拆 Vector 进 Properties 让 MPB.SetVector 100% 覆盖（material property），vert 内重组 float4x4。
        // 默认 4 列 = identity（pure 节点不走 OBJECT_MATRIX 路径，值无关）。
        _ObjM0 ("ObjM0", Vector) = (1,0,0,0)
        _ObjM1 ("ObjM1", Vector) = (0,1,0,0)
        _ObjM2 ("ObjM2", Vector) = (0,0,1,0)
        _ObjM3 ("ObjM3", Vector) = (0,0,0,1)
        // ColorFilter 矩阵（program=3，MPB 覆盖，同 _ObjM 模式）。
        // _CF0..3 = Matrix4x4 4 行（前 16 float），_CFOff = offset（第 5 列）。
        _CF0 ("CF0", Vector) = (1,0,0,0)
        _CF1 ("CF1", Vector) = (0,1,0,0)
        _CF2 ("CF2", Vector) = (0,0,1,0)
        _CF3 ("CF3", Vector) = (0,0,0,1)
        _CFOff ("CFOff", Vector) = (0,0,0,0)
    }
    SubShader
    {
        Tags { "RenderPipeline" = "UniversalPipeline" "Queue" = "Transparent" "RenderType" = "Transparent" }
        Cull Off
        ZWrite Off
        Blend [_SrcFactor] [_DstFactor]

        Pass
        {
            HLSLPROGRAM
            #pragma vertex vert
            #pragma fragment frag
            #pragma multi_compile _ CLIPPED
            #pragma multi_compile _ OBJECT_MATRIX
            #pragma multi_compile _ ALPHA_MASK
            #pragma multi_compile _ BG_COMPOSITE
            #pragma multi_compile _ COLOR_FILTER
            #include "Packages/com.unity.render-pipelines.universal/ShaderLibrary/Core.hlsl"

            struct Attr { float4 pos : POSITION; float4 color : COLOR; float2 uv : TEXCOORD0; };
            struct Vary { float4 pos : SV_POSITION; float4 color : COLOR; float2 uv : TEXCOORD0;
                          float2 clipPos : TEXCOORD1; };

            CBUFFER_START(UnityPerMaterial)
                float4 _MainTex_ST;
                float4 _ClipBox;
                // _ObjectMatrix 拆 4 Vector（Properties 对应，MPB 覆盖）。列主序：重组 float4x4(_ObjM0..3)。
                float4 _ObjM0;
                float4 _ObjM1;
                float4 _ObjM2;
                float4 _ObjM3;
                float4 _CF0;
                float4 _CF1;
                float4 _CF2;
                float4 _CF3;
                float4 _CFOff;
            CBUFFER_END
            TEXTURE2D(_MainTex); SAMPLER(sampler_MainTex);

            Vary vert(Attr v) {
                Vary o;
                // 两路径统一经 TransformObjectToWorld：GO 是 root 子 → ObjectToWorld = root_ObjectToWorld
                // （把 design world → Unity world，含 sf 缩放 + y-flip + rootPos）。
#if defined(OBJECT_MATRIX)
                // _ObjectMatrix（4 Vector 重组）把 box-local 顶点 → design world；再 TransformObjectToWorld → Unity world。
                // 直接 TransformWorldToHClip(designWorld) 漏 root transform（design 坐标 ≠ Unity world），
                // 非纯平移节点会位置/翻转/缩放全错，且与命中（design world matrix 逆投）不一致 → 点不到。
                float4x4 objM = float4x4(_ObjM0, _ObjM1, _ObjM2, _ObjM3);
                float3 designWorld = mul(objM, float4(v.pos.xy, 0, 1)).xyz;
                float3 worldPos = TransformObjectToWorld(designWorld);
#else
                float3 worldPos = TransformObjectToWorld(v.pos.xyz);
#endif
                o.pos = TransformWorldToHClip(worldPos);
                float2 clipWorldXY = worldPos.xy;
                o.color = v.color;
                o.uv = TRANSFORM_TEX(v.uv, _MainTex);
#if defined(CLIPPED)
                o.clipPos = clipWorldXY * _ClipBox.zw + _ClipBox.xy;
#endif
                return o;
            }
            half4 frag(Vary i) : SV_Target {
                // vertex color 来自 CSS（sRGB 编码）；Linear 项目 Unity 不自动转 vertex color → 须手动 sRGB→linear，
                // 否则颜色偏浅/灰蒙蒙（#1a1d2e sRGB 0.10 当 linear 显示 ~0.35）。texture 是 sRGB format 自动转，不重复。alpha 线性不转。
                half4 vcol = i.color;
                // sRGB → linear（精确 sRGB 公式；CSS 颜色 sRGB，Linear 项目 Unity 不自动转 vertex color）。
                half3 sc = vcol.rgb;
                vcol.rgb = (sc <= 0.04045) ? sc / 12.92 : pow((sc + 0.055) / 1.055, 2.4);
                half4 tex = SAMPLE_TEXTURE2D(_MainTex, sampler_MainTex, i.uv);
                #if defined(ALPHA_MASK)
                // text（program:1）：font atlas 是 alpha-mask（glyph 在 alpha，rgb 黑）→ rgb 用 vcol，alpha = vcol.a * tex.a。
                half4 col = half4(vcol.rgb, vcol.a * tex.a);
                #elif defined(BG_COMPOSITE)
                // Container+bg-image（program:2/4）：CSS background 合成 = 图(tex) over 底色(vcol)，结果直通配合 SrcAlpha blend。
                // 旧 col.a=vcol.a：无 bg-color(vcol.a=0)时全透明丢图（验收 §3.6第4/§3.7/§3.9 图消失）。
                // 标准 source-over：a=tex.a+vcol.a·(1−tex.a)；rgb 直通=预乘/a（max 防除零；a=0 像素 Blend 不贡献，rgb 无关）。
                // 有底色不透明(vcol.a=1)：a=1, rgb=图叠底色（与旧公式完全一致，零回归）。
                // 无底色(vcol.a=0)：a=tex.a, rgb=tex.rgb（等价 program:0 图直通，图显透明区透下层）。
                float bgA = tex.a + vcol.a * (1.0 - tex.a);
                float3 bgRgb = ((float3)tex.rgb * tex.a + (float3)vcol.rgb * vcol.a * (1.0 - tex.a)) / max(bgA, 1e-6);
                half4 col = half4(bgRgb, bgA);
                #else
                // image/mesh（program:0）：彩色 texture → tex.rgb × vcol。
                half4 col = tex * vcol;
                #endif
                #if defined(COLOR_FILTER)
                // CSS filter 定义在 sRGB 空间（矩阵 offset 如 contrast -0.25 = sRGB 中点 0.5 的偏移）。
                // col.rgb 当前 linear → linear→sRGB → 矩阵 → sRGB→linear，中点/色相才与浏览器对齐。
                // max(.,0) 防 pow 负底数 NaN（矩阵可出负值或超 1，最终 Blend 输出时再裁）。cfs 避免与上方 sc 重名。
                half3 cfs = col.rgb;
                cfs = (cfs <= 0.0031308) ? cfs * 12.92 : 1.055 * pow(max(cfs, 0.0), 1.0 / 2.4) - 0.055;
                float4x4 cfM = float4x4(_CF0, _CF1, _CF2, _CF3);
                cfs = mul(cfM, float4(cfs, 1.0)).rgb + _CFOff.rgb;
                cfs = (cfs <= 0.04045) ? cfs / 12.92 : pow(max((cfs + 0.055) / 1.055, 0.0), 2.4);
                col.rgb = cfs;
                #endif
                #ifdef CLIPPED
                float2 f = abs(i.clipPos);
                col.a *= step(max(f.x, f.y), 1.0);
                #endif
                return col;
            }
            ENDHLSL
        }
    }
}
