#version 100
precision highp float;
precision highp int;

struct cbMain
{
    vec3 qt_cameraPosition;
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

uniform cbMain _133;

varying vec3 qt_varWorldPos;
varying vec3 qt_varNormal;

vec3 qt_F0_ior(float ior, float metalness, vec3 baseColor)
{
    float f0 = ((ior - 1.0) * (ior - 1.0)) / ((ior + 1.0) * (ior + 1.0));
    return (vec3(f0) * (1.0 - metalness)) + (baseColor * metalness);
}

vec3 qt_principledMaterialFresnel(vec3 N, vec3 viewDir, vec3 f0, float roughness, float fresnelPower)
{
    float nDotV = clamp(dot(N, viewDir), 0.0, 1.0);
    vec3 F = f0 + ((max(vec3(1.0 - roughness), f0) - f0) * pow(1.0 - nDotV, fresnelPower));
    return F;
}

vec3 qt_linearTosRGB(vec3 color)
{
    vec3 S1 = sqrt(color);
    vec3 S2 = sqrt(S1);
    vec3 S3 = sqrt(S2);
    return ((S1 * 0.585122406482696533203125) + (S2 * 0.783140361309051513671875)) - (S3 * 0.3682627379894256591796875);
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
    float qt_objectOpacity = _133.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec3 qt_view_vector = normalize(_133.qt_cameraPosition - qt_varWorldPos);
    vec3 qt_world_normal = normalize(qt_varNormal);
    vec3 qt_tangent = vec3(0.0);
    vec3 qt_binormal = vec3(0.0);
    qt_binormal = cross(qt_world_normal, qt_tangent);
    vec4 qt_diffuseColor = _133.qt_material_base_color * qt_vertColor;
    vec3 qt_global_emission = _133.qt_material_emissive_color;
    float qt_iOR = _133.qt_material_specular.w;
    vec3 qt_specularTint = _133.qt_material_specular.xyz;
    float qt_aoFactor = 1.0;
    float qt_roughnessAmount = _133.qt_material_properties.y;
    float qt_metalnessAmount = _133.qt_material_properties.z;
    vec3 qt_specularBase = vec3(1.0);
    float qt_specularFactor = _133.qt_material_properties.x;
    vec4 global_diffuse_light = vec4(0.0);
    global_diffuse_light = vec4((_133.qt_light_ambient_total * (1.0 - qt_metalnessAmount)) * qt_diffuseColor.xyz, 0.0);
    vec3 global_specular_light = vec3(0.0);
    vec3 qt_specularAmount = qt_specularBase * vec3(qt_metalnessAmount + (qt_specularFactor * (1.0 - qt_metalnessAmount)));
    vec3 qt_f0 = vec3(1.0);
    vec3 qt_f90 = vec3(1.0);
    float param = qt_iOR;
    float param_1 = qt_metalnessAmount;
    vec3 param_2 = qt_diffuseColor.xyz;
    qt_f0 = qt_F0_ior(param, param_1, param_2);
    float qt_fresnelPower = _133.qt_material_properties2.x;
    vec3 param_3 = qt_world_normal;
    vec3 param_4 = qt_view_vector;
    vec3 param_5 = qt_f0;
    float param_6 = qt_roughnessAmount;
    float param_7 = qt_fresnelPower;
    vec3 qt_principledMaterialFresnelValue = qt_principledMaterialFresnel(param_3, param_4, param_5, param_6, param_7);
    qt_specularAmount *= qt_principledMaterialFresnelValue;
    qt_specularTint = mix(vec3(1.0), qt_specularTint, vec3(1.0 - qt_metalnessAmount));
    global_diffuse_light = vec4(global_diffuse_light.xyz * qt_aoFactor, qt_objectOpacity * qt_diffuseColor.w);
    vec4 _270 = global_diffuse_light;
    vec3 _272 = _270.xyz * (1.0 - qt_metalnessAmount);
    global_diffuse_light.x = _272.x;
    global_diffuse_light.y = _272.y;
    global_diffuse_light.z = _272.z;
    vec4 qt_color_sum = vec4((global_diffuse_light.xyz + global_specular_light) + qt_global_emission, global_diffuse_light.w);
    vec4 param_8 = qt_color_sum;
    gl_FragData[0] = qt_tonemap(param_8);
}

