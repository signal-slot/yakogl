#version 300 es

struct cbMain
{
    vec3 qt_BaseColorMap_offsets;
    vec4 qt_BaseColorMap_rotations;
    vec3 qt_cameraPosition;
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

uniform cbMain _68;

uniform highp sampler2D qt_BaseColorMap_sampler;
uniform highp sampler2D qt_NormalMap_sampler;
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
out vec2 qt_varTexCoord0;
out vec2 qt_BaseColorMap_uv_coords1;

vec2 qt_getTransformedUVCoords(vec3 inCoords, vec3 inUTransform, vec3 inVTransform)
{
    return vec2(dot(inUTransform, inCoords), dot(inVTransform, inCoords));
}

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
    gl_Position = _68.qt_modelViewProjection * qt_vertPosition;
    vec3 qt_local_model_world_position = (_68.qt_modelMatrix * qt_vertPosition).xyz;
    qt_varWorldPos = qt_local_model_world_position;
    vec3 qt_world_normal = normalize(_68.qt_normalMatrix * qt_vertNormal);
    qt_varNormal = qt_world_normal;
    qt_varTangent = (_68.qt_modelMatrix * vec4(qt_vertTangent, 0.0)).xyz;
    qt_varBinormal = (_68.qt_modelMatrix * vec4(qt_vertBinormal, 0.0)).xyz;
    qt_varTexCoord0 = qt_vertUV0;
    vec3 qt_uTransform = vec3(_68.qt_BaseColorMap_rotations.x, _68.qt_BaseColorMap_rotations.y, _68.qt_BaseColorMap_offsets.x);
    vec3 qt_vTransform = vec3(_68.qt_BaseColorMap_rotations.z, _68.qt_BaseColorMap_rotations.w, _68.qt_BaseColorMap_offsets.y);
    vec3 param = vec3(qt_varTexCoord0, 1.0);
    vec3 param_1 = qt_uTransform;
    vec3 param_2 = qt_vTransform;
    vec2 qt_BaseColorMap_uv_coords2 = qt_getTransformedUVCoords(param, param_1, param_2);
    qt_BaseColorMap_uv_coords1 = qt_BaseColorMap_uv_coords2;
}

