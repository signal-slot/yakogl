#version 300 es
precision highp float;
precision highp int;

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

uniform cbMain _308;

uniform highp samplerCube qt_lightProbe;
uniform highp sampler2D qt_NormalMap_sampler;
uniform highp sampler2D qt_BaseColorMap_sampler;

in vec3 qt_varWorldPos;
in vec3 qt_varNormal;
in vec2 qt_varTexCoord0;
layout(location = 0) out vec4 fragOutput;

vec3 qt_sampleNormalTexture(highp sampler2D inSampler, float factor, vec2 texCoord, vec3 tangent, vec3 binormal, vec3 normal)
{
    mat3 tanFrame = mat3(vec3(tangent), vec3(binormal), vec3(normal));
    vec3 tsNormal = (texture(inSampler, texCoord).xyz * 2.0) - vec3(1.0);
    tsNormal *= vec3(factor, factor, 1.0);
    return normalize(tanFrame * tsNormal);
}

vec3 qt_sRGBToLinear(vec3 srgbIn)
{
    return srgbIn * ((srgbIn * ((srgbIn * 0.30530601739883423) + vec3(0.682171106338501))) + vec3(0.012522878125309944));
}

vec4 qt_sRGBToLinear(vec4 srgbIn)
{
    vec3 param = srgbIn.xyz;
    return vec4(qt_sRGBToLinear(param), srgbIn.w);
}

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

vec3 qt_textureProbe(highp samplerCube lightProbe, vec3 dir, float lod)
{
    return textureLod(lightProbe, dir, lod).xyz;
}

vec3 qt_exposure(vec3 color, float exposure)
{
    return vec3(1.0) - exp((-color) * exposure);
}

vec4 qt_sampleDiffuse(vec3 normal)
{
    if (_308.qt_lightProbeProperties.w < 0.004999999888241291)
    {
        return vec4(0.0);
    }
    vec3 smpDir = normal;
    float baseLevel = _308.qt_lightProbeProperties.y;
    vec3 param = smpDir;
    float param_1 = baseLevel;
    vec3 val = qt_textureProbe(qt_lightProbe, param, param_1);
    if (_308.qt_lightProbeProperties.z > (-1.0))
    {
        float ctr = 0.5 + (0.5 * _308.qt_lightProbeProperties.z);
        float vertWt = smoothstep(ctr * 0.25, ctr + 0.25, smpDir.y);
        float wtScaled = mix(1.0, vertWt, _308.qt_lightProbeProperties.z + 1.0);
        val *= wtScaled;
    }
    vec3 param_2 = val;
    float param_3 = _308.qt_lightProbeProperties.w;
    return vec4(qt_exposure(param_2, param_3), 1.0);
}

vec2 qt_brdfApproximation(vec3 N, vec3 viewDir, float roughness)
{
    float nDotV = clamp(dot(N, viewDir), 0.0, 1.0);
    vec4 r = (vec4(-1.0, -0.027499999850988388, -0.5720000267028809, 0.02199999988079071) * roughness) + vec4(1.0, 0.042500000447034836, 1.0399999618530273, -0.03999999910593033);
    float a004 = (min(r.x * r.x, exp2((-9.279999732971191) * nDotV)) * r.x) + r.y;
    return (vec2(-1.0399999618530273, 1.0399999618530273) * a004) + r.zw;
}

vec4 qt_sampleGlossyPrincipled(vec3 normal, vec3 viewDir, vec3 F, float roughness)
{
    if (_308.qt_lightProbeProperties.w < 0.004999999888241291)
    {
        return vec4(0.0);
    }
    float levels = _308.qt_lightProbeProperties.y - 1.0;
    float lod = clamp(roughness * 5.0, 0.0, levels);
    vec3 smpDir = normalize(reflect(-viewDir, normal));
    vec3 param = smpDir;
    float param_1 = lod;
    vec3 specularSample = qt_textureProbe(qt_lightProbe, param, param_1);
    vec3 param_2 = normal;
    vec3 param_3 = viewDir;
    float param_4 = roughness;
    vec2 brdf = qt_brdfApproximation(param_2, param_3, param_4);
    if (_308.qt_lightProbeProperties.z > (-1.0))
    {
        float ctr = 0.5 + (0.5 * _308.qt_lightProbeProperties.z);
        float vertWt = smoothstep(ctr * 0.25, ctr + 0.25, smpDir.y);
        float wtScaled = mix(1.0, vertWt, _308.qt_lightProbeProperties.z + 1.0);
        specularSample *= wtScaled;
    }
    vec3 sampleColor = specularSample * ((F * brdf.x) + vec3(brdf.y));
    vec3 param_5 = sampleColor;
    float param_6 = _308.qt_lightProbeProperties.w;
    return vec4(qt_exposure(param_5, param_6), 1.0);
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
    float qt_objectOpacity = _308.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec3 qt_view_vector = normalize(_308.qt_cameraPosition - qt_varWorldPos);
    vec3 qt_world_normal = normalize(qt_varNormal);
    vec3 qt_tangent = vec3(0.0);
    vec3 qt_binormal = vec3(0.0);
    vec2 qt_texCoord0 = qt_varTexCoord0;
    vec2 dUVdx = dFdx(qt_texCoord0);
    vec2 dUVdy = dFdy(qt_texCoord0);
    qt_tangent = ((dFdx(qt_varWorldPos) * dUVdy.y) - (dFdy(qt_varWorldPos) * dUVdx.y)) / vec3((dUVdx.x * dUVdy.y) - (dUVdx.y * dUVdy.x));
    qt_tangent -= (qt_world_normal * dot(qt_world_normal, qt_tangent));
    qt_tangent = normalize(qt_tangent);
    qt_binormal = cross(qt_world_normal, qt_tangent);
    float qt_facing = gl_FrontFacing ? 1.0 : (-1.0);
    qt_world_normal *= qt_facing;
    qt_tangent *= qt_facing;
    qt_binormal *= qt_facing;
    vec4 qt_diffuseColor = _308.qt_material_base_color * qt_vertColor;
    vec3 qt_global_emission = _308.qt_material_emissive_color;
    float qt_iOR = _308.qt_material_specular.w;
    float qt_normalStrength = _308.qt_material_properties2.y;
    float param = qt_normalStrength;
    vec2 param_1 = qt_texCoord0;
    vec3 param_2 = qt_tangent;
    vec3 param_3 = qt_binormal;
    vec3 param_4 = qt_world_normal;
    qt_world_normal = qt_sampleNormalTexture(qt_NormalMap_sampler, param, param_1, param_2, param_3, param_4);
    vec3 qt_specularTint = _308.qt_material_specular.xyz;
    vec4 qt_base_texture_color = texture(qt_BaseColorMap_sampler, qt_texCoord0);
    vec4 param_5 = qt_base_texture_color;
    qt_base_texture_color = qt_sRGBToLinear(param_5);
    qt_diffuseColor *= qt_base_texture_color;
    qt_diffuseColor.w = 1.0;
    float qt_aoFactor = 1.0;
    float qt_roughnessAmount = _308.qt_material_properties.y;
    float qt_metalnessAmount = _308.qt_material_properties.z;
    vec3 qt_specularBase = vec3(1.0);
    float qt_specularFactor = _308.qt_material_properties.x;
    vec4 global_diffuse_light = vec4(0.0);
    global_diffuse_light = vec4((_308.qt_light_ambient_total * (1.0 - qt_metalnessAmount)) * qt_diffuseColor.xyz, 0.0);
    vec3 global_specular_light = vec3(0.0);
    vec3 qt_specularAmount = qt_specularBase * vec3(qt_metalnessAmount + (qt_specularFactor * (1.0 - qt_metalnessAmount)));
    vec3 qt_f0 = vec3(1.0);
    vec3 qt_f90 = vec3(1.0);
    float param_6 = qt_iOR;
    float param_7 = qt_metalnessAmount;
    vec3 param_8 = qt_diffuseColor.xyz;
    qt_f0 = qt_F0_ior(param_6, param_7, param_8);
    float qt_fresnelPower = _308.qt_material_properties2.x;
    vec3 param_9 = qt_world_normal;
    vec3 param_10 = qt_view_vector;
    vec3 param_11 = qt_f0;
    float param_12 = qt_roughnessAmount;
    float param_13 = qt_fresnelPower;
    vec3 qt_principledMaterialFresnelValue = qt_principledMaterialFresnel(param_9, param_10, param_11, param_12, param_13);
    qt_specularAmount *= qt_principledMaterialFresnelValue;
    qt_specularTint = mix(vec3(1.0), qt_specularTint, vec3(1.0 - qt_metalnessAmount));
    global_diffuse_light = vec4(global_diffuse_light.xyz * qt_aoFactor, qt_objectOpacity * qt_diffuseColor.w);
    vec3 param_14 = qt_world_normal;
    vec3 qt_iblDiffuse = (qt_diffuseColor.xyz * (vec3(1.0) - qt_specularAmount)) * qt_sampleDiffuse(param_14).xyz;
    vec3 param_15 = qt_world_normal;
    vec3 param_16 = qt_view_vector;
    vec3 param_17 = qt_specularAmount;
    float param_18 = qt_roughnessAmount;
    vec3 qt_iblSpecular = qt_specularTint * qt_sampleGlossyPrincipled(param_15, param_16, param_17, param_18).xyz;
    vec4 _701 = global_diffuse_light;
    vec3 _703 = _701.xyz + (qt_iblDiffuse * qt_aoFactor);
    global_diffuse_light.x = _703.x;
    global_diffuse_light.y = _703.y;
    global_diffuse_light.z = _703.z;
    global_specular_light += (qt_iblSpecular * qt_aoFactor);
    vec4 _717 = global_diffuse_light;
    vec3 _719 = _717.xyz * (1.0 - qt_metalnessAmount);
    global_diffuse_light.x = _719.x;
    global_diffuse_light.y = _719.y;
    global_diffuse_light.z = _719.z;
    vec4 qt_color_sum = vec4((global_diffuse_light.xyz + global_specular_light) + qt_global_emission, global_diffuse_light.w);
    vec4 param_19 = qt_color_sum;
    fragOutput = qt_tonemap(param_19);
}

