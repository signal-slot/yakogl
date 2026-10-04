#version 300 es
precision highp float;
precision highp int;

struct cbMain
{
    vec3 qt_cameraPosition;
    vec2 qt_cameraProperties;
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
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
};

uniform cbMain _15;

in vec3 qt_varShadowWorldPos;
layout(location = 0) out vec4 fragOutput;

void main()
{
    float qt_objectOpacity = _15.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec4 qt_diffuseColor = _15.qt_material_base_color * qt_vertColor;
    vec3 qt_global_emission = _15.qt_material_emissive_color;
    float qt_iOR = _15.qt_material_specular.w;
    vec3 qt_specularTint = _15.qt_material_specular.xyz;
    vec3 qt_shadowCamPos = vec3(_15.qt_cameraPosition.x, _15.qt_cameraPosition.y, _15.qt_cameraPosition.z);
    float qt_shadowDist = length(qt_varShadowWorldPos - qt_shadowCamPos);
    qt_shadowDist = (qt_shadowDist - _15.qt_cameraProperties.x) / (_15.qt_cameraProperties.y - _15.qt_cameraProperties.x);
    fragOutput = vec4(qt_shadowDist, qt_shadowDist, qt_shadowDist, 1.0);
}

