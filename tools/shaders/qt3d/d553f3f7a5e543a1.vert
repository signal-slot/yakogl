#version 300 es

struct cbMain
{
    vec3 qt_cameraPosition;
    vec2 qt_cameraProperties;
    vec4 qt_fogColor;
    vec4 qt_fogDepthProperties;
    vec4 qt_fogHeightProperties;
    vec4 qt_fogTransmitProperties;
    vec4 qt_lightAndShadowCounts;
    vec3 qt_light_ambient_total;
    vec4 qt_material_base_color;
    float qt_material_clearcoat_fresnel_power;
    float qt_material_clearcoat_normal_strength;
    vec3 qt_material_emissive_color;
    vec4 qt_material_properties;
    vec4 qt_material_properties2;
    vec4 qt_material_properties3;
    vec4 qt_material_specular;
    mat4 qt_modelMatrix;
    mat4 qt_modelViewProjection;
    mat3 qt_normalMatrix;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
    mat4 qt_viewProjectionMatrix;
};

uniform cbMain _53;

uniform highp sampler2D qt_screenTexture;
uniform highp sampler2D qt_shadowmap_blue_noise_texture;
uniform highp sampler2DArray qt_shadowmap_texture;

layout(location = 2) in vec3 attr_pos;
layout(location = 1) in vec3 attr_norm;
layout(location = 4) in vec2 attr_uv0;
layout(location = 5) in vec2 attr_uv1;
layout(location = 3) in vec3 attr_textan;
layout(location = 0) in vec3 attr_binormal;
out vec3 qt_varWorldPos;
out vec3 qt_varNormal;
out vec3 qt_varTangent;
out vec3 qt_varBinormal;

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
    gl_Position = _53.qt_modelViewProjection * qt_vertPosition;
    vec3 qt_local_model_world_position = (_53.qt_modelMatrix * qt_vertPosition).xyz;
    qt_varWorldPos = qt_local_model_world_position;
    vec3 qt_world_normal = normalize(_53.qt_normalMatrix * qt_vertNormal);
    qt_varNormal = qt_world_normal;
    qt_varTangent = (_53.qt_modelMatrix * vec4(qt_vertTangent, 0.0)).xyz;
    qt_varBinormal = (_53.qt_modelMatrix * vec4(qt_vertBinormal, 0.0)).xyz;
}

