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
                // Container+bg-image（program:2，坑 79）：CSS background 合成。
                // 图不透明区显图（tex.rgb），透明区显 bg-color（vcol.rgb）；整体 alpha 由 bg-color 决定。
                // tex.rgb 与 vcol.rgb 均已 linear（tex sRGB 自动转，vcol 上方手动转），合成在 linear 空间正确。
                half4 col = half4(tex.rgb * tex.a + vcol.rgb * (1.0 - tex.a), vcol.a);
                #else
                // image/mesh（program:0）：彩色 texture → tex.rgb × vcol。
                half4 col = tex * vcol;
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
