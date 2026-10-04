#version 300 es

struct buf
{
    vec4 baseColorLinear;
    vec3 emissiveFactor;
    int flipY;
    int hasBaseColorMap;
    int hasEmissiveMap;
    int hasNormalMap;
    float bumpAmount;
    float regionMinU;
    float regionMinV;
    float regionMaxU;
    float regionMaxV;
};

uniform buf _15;

out vec3 v_pos;
layout(location = 0) in vec3 attr_pos;
out vec3 v_normal;
layout(location = 1) in vec3 attr_normal;
out vec2 v_uv;
out vec3 v_tangent;
out vec3 v_binormal;
layout(location = 2) in vec2 attr_lightmap_uv;

void main()
{
    vec2 regionMin = vec2(_15.regionMinU, _15.regionMinV);
    vec2 regionMax = vec2(_15.regionMaxU, _15.regionMaxV);
    v_pos = attr_pos;
    v_normal = attr_normal;
    v_uv = vec2(0.0);
    v_tangent = vec3(0.0);
    v_binormal = vec3(0.0);
    vec2 uv = attr_lightmap_uv;
    uv = (uv - regionMin) / (regionMax - regionMin);
    vec2 uv_ndc = vec2((uv.x * 2.0) - 1.0, (uv.y * 2.0) - 1.0);
    if (_15.flipY != 0)
    {
        uv_ndc.y *= (-1.0);
    }
    gl_Position = vec4(uv_ndc, 0.0, 1.0);
}

