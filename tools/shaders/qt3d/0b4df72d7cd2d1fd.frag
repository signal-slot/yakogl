#version 300 es
precision highp float;
precision highp int;

struct cbMain
{
    vec3 qt_cameraPosition;
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

uniform cbMain _15;

in vec3 qt_varWorldPos;
in vec3 qt_varNormal;
layout(location = 0) out vec4 GBUFFER0;
layout(location = 1) out vec4 GBUFFER1;
layout(location = 2) out vec4 GBUFFER2;

void main()
{
    float qt_objectOpacity = _15.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec3 qt_view_vector = normalize(_15.qt_cameraPosition - qt_varWorldPos);
    vec3 qt_world_normal = normalize(qt_varNormal);
    vec4 qt_diffuseColor = _15.qt_material_base_color * qt_vertColor;
    vec3 qt_global_emission = _15.qt_material_emissive_color;
    float qt_iOR = _15.qt_material_specular.w;
    vec3 qt_specularTint = _15.qt_material_specular.xyz;
    float qt_roughnessAmount = _15.qt_material_properties.y;
    float qt_metalnessAmount = _15.qt_material_properties.z;
    vec3 baseColor = qt_diffuseColor.xyz;
    float metalness = qt_metalnessAmount;
    float roughness = qt_roughnessAmount;
    vec3 worldNormal = normalize(qt_world_normal);
    GBUFFER0 = vec4(baseColor, metalness);
    GBUFFER1 = vec4((worldNormal * 0.5) + vec3(0.5), roughness);
    GBUFFER2 = vec4(qt_varWorldPos, 1.0);
}

