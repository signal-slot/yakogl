#version 100
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

uniform cbMain _104;

uniform highp sampler2D qt_BaseColorMap_sampler;

varying vec3 qt_varWorldPos;
varying vec3 qt_varNormal;
varying vec3 qt_varTangent;
varying vec3 qt_varBinormal;
varying vec2 qt_varTexCoord0;

vec3 qt_sRGBToLinear(vec3 srgbIn)
{
    return srgbIn * ((srgbIn * ((srgbIn * 0.30530601739883423) + vec3(0.682171106338501))) + vec3(0.012522878125309944));
}

vec4 qt_sRGBToLinear(vec4 srgbIn)
{
    vec3 param = srgbIn.xyz;
    return vec4(qt_sRGBToLinear(param), srgbIn.w);
}

vec3 qt_linearTosRGB(vec3 color)
{
    vec3 S1 = sqrt(color);
    vec3 S2 = sqrt(S1);
    vec3 S3 = sqrt(S2);
    return ((S1 * 0.5851224064826965) + (S2 * 0.7831403613090515)) - (S3 * 0.36826273798942566);
}

vec3 qt_tonemap(vec3 color)
{
    vec3 param = color;
    return qt_linearTosRGB(param);
}

vec4 qt_tonemap(vec4 color)
{
    vec3 param = color.xyz;
    return vec4(qt_tonemap(param), color.w);
}

void main()
{
    float qt_objectOpacity = _104.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec3 qt_view_vector = normalize(_104.qt_cameraPosition - qt_varWorldPos);
    vec3 qt_world_normal = normalize(qt_varNormal);
    vec3 qt_tangent = normalize(qt_varTangent);
    vec3 qt_binormal = normalize(qt_varBinormal);
    vec4 qt_diffuseColor = _104.qt_material_base_color * qt_vertColor;
    vec3 qt_global_emission = _104.qt_material_emissive_color;
    float qt_iOR = _104.qt_material_specular.w;
    vec3 qt_specularTint = _104.qt_material_specular.xyz;
    vec2 qt_texCoord0 = qt_varTexCoord0;
    vec4 qt_base_texture_color = texture2D(qt_BaseColorMap_sampler, qt_texCoord0);
    vec4 param = qt_base_texture_color;
    qt_base_texture_color = qt_sRGBToLinear(param);
    qt_diffuseColor *= qt_base_texture_color;
    float qt_aoFactor = 1.0;
    float qt_roughnessAmount = _104.qt_material_properties.y;
    float qt_metalnessAmount = _104.qt_material_properties.z;
    vec4 qt_color_sum = vec4(qt_diffuseColor.xyz, qt_diffuseColor.w * qt_objectOpacity);
    vec4 param_1 = qt_color_sum;
    gl_FragData[0] = qt_tonemap(param_1);
}

