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
    mat4 qt_parentMatrix;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
    mat4 qt_viewProjectionMatrix;
};

uniform cbMain _75;

uniform highp sampler2D qt_screenTexture;
uniform highp sampler2D qt_shadowmap_blue_noise_texture;
uniform highp sampler2DArray qt_shadowmap_texture;

layout(location = 1) in vec3 attr_pos;
layout(location = 0) in vec3 attr_norm;
layout(location = 2) in vec2 attr_uv0;
layout(location = 3) in vec4 qt_instanceColor;
layout(location = 5) in vec4 qt_instanceTransform0;
layout(location = 6) in vec4 qt_instanceTransform1;
layout(location = 7) in vec4 qt_instanceTransform2;
out vec4 qt_varColor;
out vec3 qt_varWorldPos;
out vec3 qt_varNormal;
layout(location = 4) in vec4 qt_instanceData;

void main()
{
    vec4 qt_vertPosition = vec4(attr_pos, 1.0);
    vec3 qt_vertNormal = vec3(0.0);
    vec3 qt_vertTangent = vec3(0.0);
    vec3 qt_vertBinormal = vec3(0.0);
    vec2 qt_vertUV0 = vec2(0.0);
    vec4 qt_vertColor = vec4(1.0);
    qt_vertNormal = attr_norm;
    qt_vertUV0 = attr_uv0;
    qt_vertColor *= qt_instanceColor;
    mat4 qt_instanceMatrix = mat4(vec4(qt_instanceTransform0), vec4(qt_instanceTransform1), vec4(qt_instanceTransform2), vec4(0.0, 0.0, 0.0, 1.0));
    mat4 qt_instancedModelMatrix = (_75.qt_parentMatrix * transpose(qt_instanceMatrix)) * _75.qt_modelMatrix;
    mat4 _93 = transpose(inverse(qt_instancedModelMatrix));
    mat3 qt_instancedNormalMatrix = mat3(_93[0].xyz, _93[1].xyz, _93[2].xyz);
    mat4 qt_instancedMVPMatrix = _75.qt_viewProjectionMatrix * qt_instancedModelMatrix;
    gl_Position = qt_instancedMVPMatrix * qt_vertPosition;
    qt_varColor = qt_vertColor;
    vec3 qt_local_model_world_position = (qt_instancedModelMatrix * qt_vertPosition).xyz;
    qt_varWorldPos = qt_local_model_world_position;
    vec3 qt_world_normal = normalize(qt_instancedNormalMatrix * qt_vertNormal);
    qt_varNormal = qt_world_normal;
}

