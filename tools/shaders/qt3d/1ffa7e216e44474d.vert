#version 300 es

struct cbMain
{
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
    float qt_morphWeights[3];
    mat3 qt_normalMatrix;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
    mat4 qt_viewProjectionMatrix;
};

uniform cbMain _91;

uniform highp sampler2DArray qt_morphTargetTexture;
uniform highp sampler2D qt_screenTexture;
uniform highp sampler2D qt_shadowmap_blue_noise_texture;
uniform highp sampler2DArray qt_shadowmap_texture;

layout(location = 1) in vec3 attr_pos;
layout(location = 0) in vec3 attr_norm;
layout(location = 2) in vec2 attr_uv0;
out vec3 qt_varWorldPos;
out vec3 qt_varNormal;

vec4 getTargetTexValue(ivec3 texCoord)
{
    return texelFetch(qt_morphTargetTexture, texCoord, 0);
}

vec4 qt_getTargetValue(vec4 orgValue, int offset)
{
    vec4 result = orgValue;
    int texWidth = textureSize(qt_morphTargetTexture, 0).x;
    ivec3 texCoord;
    texCoord.x = gl_VertexID % texWidth;
    texCoord.y = (gl_VertexID - texCoord.x) / texWidth;
    for (int i = 0; i < 3; i++)
    {
        texCoord.z = offset + i;
        ivec3 param = texCoord;
        vec4 targetValue = getTargetTexValue(param);
        result += ((targetValue - orgValue) * _91.qt_morphWeights[i]);
    }
    return result;
}

vec3 qt_getTargetPosition(vec3 pos)
{
    vec4 param = vec4(pos, 1.0);
    int param_1 = 0;
    return qt_getTargetValue(param, param_1).xyz;
}

vec3 qt_getTargetNormal(vec3 norm)
{
    vec4 param = vec4(norm, 1.0);
    int param_1 = 3;
    return qt_getTargetValue(param, param_1).xyz;
}

void main()
{
    vec4 qt_vertPosition = vec4(attr_pos, 1.0);
    vec3 qt_vertNormal = vec3(0.0);
    vec3 qt_vertTangent = vec3(0.0);
    vec3 qt_vertBinormal = vec3(0.0);
    vec2 qt_vertUV0 = vec2(0.0);
    qt_vertNormal = attr_norm;
    qt_vertUV0 = attr_uv0;
    vec3 param = qt_vertPosition.xyz;
    vec3 _157 = qt_getTargetPosition(param);
    qt_vertPosition.x = _157.x;
    qt_vertPosition.y = _157.y;
    qt_vertPosition.z = _157.z;
    gl_Position = _91.qt_modelViewProjection * qt_vertPosition;
    vec3 qt_local_model_world_position = (_91.qt_modelMatrix * qt_vertPosition).xyz;
    qt_varWorldPos = qt_local_model_world_position;
    vec3 param_1 = qt_vertNormal;
    qt_vertNormal = qt_getTargetNormal(param_1);
    vec3 qt_world_normal = normalize(_91.qt_normalMatrix * qt_vertNormal);
    qt_varNormal = qt_world_normal;
}

