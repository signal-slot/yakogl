#version 300 es
precision highp float;
precision highp int;

struct cbMain
{
    vec4 qt_material_base_color;
    float qt_material_clearcoat_fresnel_power;
    float qt_material_clearcoat_normal_strength;
    vec3 qt_material_emissive_color;
    vec4 qt_material_properties;
    vec4 qt_material_properties2;
    vec4 qt_material_properties3;
    vec4 qt_material_specular;
    mat4 qt_modelViewProjection;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
};

uniform cbMain _14;

void main()
{
    float qt_objectOpacity = _14.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec4 qt_diffuseColor = _14.qt_material_base_color * qt_vertColor;
    vec3 qt_global_emission = _14.qt_material_emissive_color;
    float qt_iOR = _14.qt_material_specular.w;
    vec3 qt_specularTint = _14.qt_material_specular.xyz;
    vec4 fragOutput = vec4(0.0);
}

