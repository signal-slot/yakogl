#version 300 es

struct cbMain
{
    vec4 qt_material_base_color;
    float qt_material_clearcoat_fresnel_power;
    float qt_material_clearcoat_normal_strength;
    vec3 qt_material_emissive_color;
    vec4 qt_material_properties;
    vec4 qt_material_properties2;
    vec4 qt_material_properties3;
    vec4 qt_material_specular;
    mat4 qt_modelViewProjection;
    vec4 qt_rhi_properties;
    vec2 qt_shadowDepthAdjust;
    mat4 qt_viewMatrix;
};

uniform cbMain _52;

layout(location = 2) in vec3 attr_pos;
layout(location = 1) in vec3 attr_norm;
layout(location = 4) in vec2 attr_uv0;
layout(location = 5) in vec2 attr_uv1;
layout(location = 3) in vec3 attr_textan;
layout(location = 0) in vec3 attr_binormal;
out float qt_varDepth;

void main()
{
    vec4 qt_vertPosition = vec4(attr_pos, 1.0);
    vec3 qt_vertNormal = vec3(0.0);
    vec3 qt_vertTangent = vec3(0.0);
    vec3 qt_vertBinormal = vec3(0.0);
    vec2 qt_vertUV0 = vec2(0.0);
    vec2 qt_vertUV1 = vec2(0.0);
    qt_vertNormal = attr_norm;
    qt_vertUV0 = attr_uv0;
    qt_vertUV1 = attr_uv1;
    qt_vertTangent = attr_textan;
    qt_vertBinormal = attr_binormal;
    gl_Position = _52.qt_modelViewProjection * qt_vertPosition;
    qt_varDepth = gl_Position.z / gl_Position.w;
}

