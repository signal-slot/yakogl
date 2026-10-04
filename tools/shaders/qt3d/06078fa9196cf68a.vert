#version 300 es

struct cbMain
{
    vec3 qt_cameraPosition;
    vec4 qt_lightProbeProperties;
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
};

uniform cbMain _46;

uniform highp samplerCube qt_lightProbe;

layout(location = 1) in vec3 attr_pos;
layout(location = 0) in vec3 attr_norm;
layout(location = 2) in vec2 attr_uv0;
out vec3 qt_varWorldPos;
out vec3 qt_varNormal;

void main()
{
    vec4 qt_vertPosition = vec4(attr_pos, 1.0);
    vec3 qt_vertNormal = vec3(0.0);
    vec3 qt_vertTangent = vec3(0.0);
    vec3 qt_vertBinormal = vec3(0.0);
    vec2 qt_vertUV0 = vec2(0.0);
    qt_vertNormal = attr_norm;
    qt_vertUV0 = attr_uv0;
    gl_Position = _46.qt_modelViewProjection * qt_vertPosition;
    vec3 qt_local_model_world_position = (_46.qt_modelMatrix * qt_vertPosition).xyz;
    qt_varWorldPos = qt_local_model_world_position;
    vec3 qt_world_normal = normalize(_46.qt_normalMatrix * qt_vertNormal);
    qt_varNormal = qt_world_normal;
}

