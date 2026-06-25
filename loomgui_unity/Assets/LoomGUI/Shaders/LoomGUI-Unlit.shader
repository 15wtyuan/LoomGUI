Shader "LoomGUI/Unlit"
{
    Properties
    {
        _MainTex ("Texture", 2D) = "white" {}
        _SrcFactor ("SrcFactor", Float) = 5   // SrcAlpha
        _DstFactor ("DstFactor", Float) = 10  // OneMinusSrcAlpha
        _ClipBox ("ClipBox", Vector) = (0,0,1,1)
        // _ObjectMatrix 不进 Properties——ShaderLab 无 Matrix property 类型（原声明致
        // "unexpected TOK_MATRIX" parse error）。由 MaterialPropertyBlock 每帧 SetMatrix
        // 覆盖（MirrorPool 非纯平移路径）；uniform 声明见下方 CBUFFER 外。
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
            #include "Packages/com.unity.render-pipelines.universal/ShaderLibrary/Core.hlsl"

            struct Attr { float4 pos : POSITION; float4 color : COLOR; float2 uv : TEXCOORD0; };
            struct Vary { float4 pos : SV_POSITION; float4 color : COLOR; float2 uv : TEXCOORD0;
                          float2 clipPos : TEXCOORD1; };

            CBUFFER_START(UnityPerMaterial)
                float4 _MainTex_ST;
                float4 _ClipBox;
                // _ObjectMatrix 在 CBUFFER（无 Properties 对应——ShaderLab 无 Matrix property 类型）。
                // 坑：放 CBUFFER 外的全局 uniform 时，MaterialPropertyBlock.SetMatrix 不覆盖
                //（MPB 只覆盖 material property，全局 uniform 不属 material）→ _ObjectMatrix 恒 0 →
                // matrix 路径顶点塌缩（v1d.4 popup 飞掉）。放回 CBUFFER 让 MPB 按 name 覆盖。
                // 代价：CBUFFER 含非 Properties 字段 → 整 shader 丢 SRP Batcher 资格（matrix 节点
                // 用 MPB 本就不 batch；v1e 用 instanced property 再优化）。
                float4x4 _ObjectMatrix;
            CBUFFER_END
            TEXTURE2D(_MainTex); SAMPLER(sampler_MainTex);

            Vary vert(Attr v) {
                Vary o;
                // 两路径统一经 TransformObjectToWorld：GO 是 root 子 → ObjectToWorld = root_ObjectToWorld
                // （把 design world → Unity world，含 sf 缩放 + y-flip + rootPos）。
#if defined(OBJECT_MATRIX)
                // _ObjectMatrix 把 box-local 顶点 → design world；再 TransformObjectToWorld → Unity world。
                // 坑：直接 TransformWorldToHClip(designWorld) 漏 root transform（design 坐标 ≠ Unity world），
                // 非纯平移节点会位置/翻转/缩放全错，且与命中（design world matrix 逆投）不一致 → 点不到。
                float3 designWorld = mul(_ObjectMatrix, float4(v.pos.xy, 0, 1)).xyz;
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
                half4 col = SAMPLE_TEXTURE2D(_MainTex, sampler_MainTex, i.uv) * i.color;
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
