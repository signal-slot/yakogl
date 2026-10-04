#version 300 es
precision highp float;
precision highp int;

struct buf
{
    vec4 baseColorLinear;
    vec3 emissiveFactor;
    int flipY;
    int hasBaseColorMap;
    int hasEmissiveMap;
    int hasNormalMap;
    float normalStrength;
};

uniform buf _63;

uniform highp sampler2D baseColorMap;
uniform highp sampler2D emissiveMap;
uniform highp sampler2D normalMap;

layout(location = 0) out vec4 position;
in vec3 v_pos;
layout(location = 2) out vec4 baseColor;
in vec2 v_uv;
layout(location = 3) out vec4 emissive;
in vec3 v_normal;
in vec3 v_tangent;
in vec3 v_binormal;
layout(location = 1) out vec4 normal;

vec3 sRGBToLinear(vec3 c)
{
    return c * ((c * ((c * 0.305306017398834228515625) + vec3(0.6821711063385009765625))) + vec3(0.01252287812530994415283203125));
}

vec4 sRGBToLinear(vec4 c)
{
    vec3 param = c.xyz;
    return vec4(sRGBToLinear(param), c.w);
}

void main()
{
    position = vec4(v_pos, 1.0);
    baseColor = _63.baseColorLinear;
    if (_63.hasBaseColorMap != 0)
    {
        vec4 param = texture(baseColorMap, v_uv);
        baseColor *= sRGBToLinear(param);
    }
    emissive = vec4(_63.emissiveFactor, 1.0);
    if (_63.hasEmissiveMap != 0)
    {
        vec3 param_1 = texture(emissiveMap, v_uv).xyz;
        vec4 _112 = emissive;
        vec3 _114 = _112.xyz * sRGBToLinear(param_1);
        emissive.x = _114.x;
        emissive.y = _114.y;
        emissive.z = _114.z;
    }
    vec3 N = normalize(v_normal);
    if (_63.hasNormalMap != 0)
    {
        vec3 tangent = normalize(v_tangent);
        vec3 binormal = normalize(v_binormal);
        mat3 tanFrame = mat3(vec3(tangent), vec3(binormal), vec3(N));
        vec3 tsNormal = (texture(normalMap, v_uv).xyz * 2.0) - vec3(1.0);
        tsNormal *= vec3(_63.normalStrength, _63.normalStrength, 1.0);
        N = tanFrame * normalize(tsNormal);
    }
    normal = vec4(N, 1.0);
}

