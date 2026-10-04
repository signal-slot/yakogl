#version 300 es

struct cbMain
{
    vec3 qt_cameraDirection;
    vec3 qt_cameraPosition;
    vec2 qt_cameraProperties;
    vec4 qt_lightAndShadowCounts;
    vec4 qt_lightProbeProperties;
    vec4 qt_material_properties;
    mat4 qt_modelMatrix;
    mat4 qt_modelViewProjection;
    mat3 qt_normalMatrix;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
    mat4 qt_viewProjectionMatrix;
};

uniform cbMain _42;

uniform highp sampler2D gbuffer0;
uniform highp sampler2D gbuffer1;
uniform highp sampler2D gbuffer2;
uniform highp samplerCube qt_lightProbe;
uniform highp sampler2D qt_screenTexture;
uniform highp sampler2D qt_shadowmap_blue_noise_texture;
uniform highp sampler2DArray qt_shadowmap_texture;

out vec2 texcoord;
layout(location = 1) in vec3 attr_pos;
layout(location = 0) in vec3 attr_norm;
layout(location = 2) in vec2 attr_uv0;

void qt_customMain(vec3 VERTEX, vec3 NORMAL, vec2 UV0, vec2 UV1, vec3 TANGENT, vec3 BINORMAL, ivec4 JOINTS, vec4 WEIGHTS, vec4 COLOR)
{
    texcoord = UV0;
    vec2 ndc = (UV0 * 2.0) - vec2(1.0);
    ndc.y *= _42.qt_rhi_properties.y;
    gl_Position = vec4(ndc.x, ndc.y, 0.0, 1.0);
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
}

