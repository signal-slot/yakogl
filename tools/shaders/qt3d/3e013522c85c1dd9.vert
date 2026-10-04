#version 300 es

struct cbMain
{
    vec3 qt_cameraDirection;
    vec3 qt_cameraPosition;
    vec2 qt_cameraProperties;
    vec4 qt_lightAndShadowCounts;
    vec4 qt_lightProbeProperties;
    vec3 qt_light_ambient_total;
    vec4 qt_material_base_color;
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
    float uAmplitude;
    vec4 uDiffuse;
    float uShininess;
    float uTime;
};

uniform cbMain _32;

uniform highp samplerCube qt_lightProbe;
uniform highp sampler2D qt_screenTexture;
uniform highp sampler2D qt_shadowmap_blue_noise_texture;
uniform highp sampler2DArray qt_shadowmap_texture;

layout(location = 2) in vec3 attr_pos;
layout(location = 1) in vec3 attr_norm;
layout(location = 4) in vec2 attr_uv0;
layout(location = 3) in vec3 attr_textan;
layout(location = 0) in vec3 attr_binormal;
out vec3 qt_varWorldPos;
out vec3 qt_varNormal;
out vec3 qt_varTangent;
out vec3 qt_varBinormal;
out vec2 qt_varTexCoord0;
out vec2 qt_varTexCoord1;

void qt_customMain(inout vec3 VERTEX, vec3 NORMAL, vec2 UV0, vec2 UV1, vec3 TANGENT, vec3 BINORMAL, ivec4 JOINTS, vec4 WEIGHTS, vec4 COLOR)
{
    VERTEX.y += (sin(_32.uTime + (VERTEX.x * 10.0)) * _32.uAmplitude);
}

void main()
{
    vec4 qt_vertPosition = vec4(attr_pos, 1.0);
    vec3 qt_vertNormal = vec3(0.0);
    vec3 qt_vertTangent = vec3(0.0);
    vec3 qt_vertBinormal = vec3(0.0);
    vec2 qt_vertUV0 = vec2(0.0);
    vec2 qt_vertUV1 = vec2(0.0);
    ivec4 qt_vertJoints = ivec4(0);
    vec4 qt_vertWeights = vec4(0.0);
    vec4 qt_vertColor = vec4(1.0);
    qt_vertNormal = attr_norm;
    qt_vertUV0 = attr_uv0;
    qt_vertTangent = attr_textan;
    qt_vertBinormal = attr_binormal;
    vec3 param = qt_vertPosition.xyz;
    vec3 param_1 = qt_vertNormal;
    vec2 param_2 = qt_vertUV0;
    vec2 param_3 = qt_vertUV1;
    vec3 param_4 = qt_vertTangent;
    vec3 param_5 = qt_vertBinormal;
    ivec4 param_6 = qt_vertJoints;
    vec4 param_7 = qt_vertWeights;
    vec4 param_8 = qt_vertColor;
    qt_customMain(param, param_1, param_2, param_3, param_4, param_5, param_6, param_7, param_8);
    qt_vertPosition.x = param.x;
    qt_vertPosition.y = param.y;
    qt_vertPosition.z = param.z;
    qt_vertNormal = param_1;
    qt_vertUV0 = param_2;
    qt_vertUV1 = param_3;
    qt_vertTangent = param_4;
    qt_vertBinormal = param_5;
    qt_vertJoints = param_6;
    qt_vertWeights = param_7;
    qt_vertColor = param_8;
    gl_Position = _32.qt_modelViewProjection * qt_vertPosition;
    vec3 qt_local_model_world_position = (_32.qt_modelMatrix * qt_vertPosition).xyz;
    qt_varWorldPos = qt_local_model_world_position;
    vec3 qt_world_normal = normalize(_32.qt_normalMatrix * qt_vertNormal);
    qt_varNormal = qt_world_normal;
    qt_varTangent = (_32.qt_modelMatrix * vec4(qt_vertTangent, 0.0)).xyz;
    qt_varBinormal = (_32.qt_modelMatrix * vec4(qt_vertBinormal, 0.0)).xyz;
    qt_varTexCoord0 = qt_vertUV0;
    qt_varTexCoord1 = qt_vertUV1;
}

