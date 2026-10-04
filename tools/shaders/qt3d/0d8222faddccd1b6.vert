#version 300 es

struct cbMain
{
    float alpha;
    float amplitude;
    vec3 qt_cameraDirection;
    vec3 qt_cameraPosition;
    vec2 qt_cameraProperties;
    vec4 qt_material_properties;
    mat4 qt_modelMatrix;
    mat4 qt_modelViewProjection;
    mat3 qt_normalMatrix;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
    mat4 qt_viewProjectionMatrix;
    uint textureFromItem;
    uint texturing;
    float time;
};

uniform cbMain _36;

uniform highp sampler2D tex;

out vec3 pos;
out vec2 coord;
layout(location = 2) in vec3 attr_pos;
layout(location = 1) in vec3 attr_norm;
layout(location = 4) in vec2 attr_uv0;
layout(location = 3) in vec3 attr_textan;
layout(location = 0) in vec3 attr_binormal;

void qt_customMain(vec3 VERTEX, vec3 NORMAL, vec2 UV0, vec2 UV1, vec3 TANGENT, vec3 BINORMAL, ivec4 JOINTS, vec4 WEIGHTS, vec4 COLOR)
{
    pos = VERTEX;
    pos.x += (sin((_36.time * 4.0) + pos.y) * _36.amplitude);
    coord = UV0;
    gl_Position = _36.qt_modelViewProjection * vec4(pos, 1.0);
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
}

