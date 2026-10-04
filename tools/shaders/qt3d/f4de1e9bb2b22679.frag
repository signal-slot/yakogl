#version 300 es
precision highp float;
precision highp int;

struct cbMain
{
    uint alwaysDirty;
    float lineWidth;
    vec3 qt_cameraDirection;
    vec3 qt_cameraPosition;
    vec2 qt_cameraProperties;
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
};

uniform cbMain _17;

uniform highp sampler2D qt_screenTexture;

in vec3 qt_varWorldPos;
in vec3 qt_varNormal;
in vec3 qt_varTangent;
in vec3 qt_varBinormal;
in vec2 qt_varTexCoord0;
in vec3 qt_varShadowWorldPos;
layout(location = 0) out vec4 fragOutput;

void main()
{
    float qt_objectOpacity = _17.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec3 qt_view_vector = normalize(_17.qt_cameraPosition - qt_varWorldPos);
    vec3 qt_world_normal = normalize(qt_varNormal);
    vec3 qt_tangent = normalize(qt_varTangent);
    vec3 qt_binormal = normalize(qt_varBinormal);
    float qt_customOcclusionAmount = 1.0;
    float qt_customIOR = 1.5;
    float qt_customSpecularAmount = 0.5;
    float qt_customSpecularRoughness = 0.0;
    float qt_customMetalnessAmount = 0.0;
    float qt_customFresnelPower = 5.0;
    vec4 qt_customBaseColor = vec4(1.0);
    vec3 qt_customEmissiveColor = vec3(0.0);
    vec2 qt_texCoord0 = qt_varTexCoord0;
    vec2 qt_texCoord1 = vec2(0.0);
    vec4 qt_diffuseColor = qt_customBaseColor * qt_vertColor;
    vec3 qt_global_emission = qt_customEmissiveColor;
    float qt_iOR = qt_customIOR;
    vec3 qt_specularTint = vec3(1.0);
    vec3 qt_shadowCamPos = vec3(_17.qt_cameraPosition.x, _17.qt_cameraPosition.y, _17.qt_cameraPosition.z);
    float qt_shadowDist = length(qt_varShadowWorldPos - qt_shadowCamPos);
    qt_shadowDist = (qt_shadowDist - _17.qt_cameraProperties.x) / (_17.qt_cameraProperties.y - _17.qt_cameraProperties.x);
    fragOutput = vec4(qt_shadowDist, qt_shadowDist, qt_shadowDist, 1.0);
}

