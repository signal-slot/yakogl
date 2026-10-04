#version 100
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
    vec2 qt_shadowDepthAdjust;
    mat4 qt_viewMatrix;
};

uniform cbMain _15;

varying float qt_varDepth;

void main()
{
    float qt_objectOpacity = _15.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec4 qt_diffuseColor = _15.qt_material_base_color * qt_vertColor;
    vec3 qt_global_emission = _15.qt_material_emissive_color;
    float qt_iOR = _15.qt_material_specular.w;
    vec3 qt_specularTint = _15.qt_material_specular.xyz;
    float qt_shadowDepth = (qt_varDepth + _15.qt_shadowDepthAdjust.x) * _15.qt_shadowDepthAdjust.y;
    gl_FragData[0] = vec4(qt_shadowDepth);
}

