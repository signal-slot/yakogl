#version 300 es
precision highp float;
precision highp int;

struct ShadowResult
{
    float illumination;
    bool isLit;
};

struct DirectionalLightData
{
    vec3 direction;
    float shadowBias;
    vec3 diffuseColor;
    float shadowFactor;
    vec3 specularColor;
    float enableShadows;
    mat4 matrices[4];
    vec4 dimensionsInverted[4];
    vec4 csmSplits;
    vec4 csmActive;
    vec4 atlasLocations[4];
    uint lightmapState;
    uint pcfSamples;
    uint csmNumSplits;
    float pcfFactor;
    float shadowMapFar;
    float csmBlendRatio;
};

struct LightData
{
    vec3 position;
    float coneAngle;
    vec3 direction;
    float innerConeAngle;
    vec3 diffuseColor;
    float constantAttenuation;
    vec3 specularColor;
    float linearAttenuation;
    uint lightmapState;
    uint pcfSamples;
    float quadraticAttenuation;
    float shadowTextureSize;
    vec2 shadowAtlasUV0;
    vec2 shadowAtlasUV1;
    float shadowAtlasLayer0;
    float shadowAtlasLayer1;
    float pcfFactor;
    float enableShadows;
    float shadowBias;
    float shadowFactor;
    float clipNear;
    float shadowMapFar;
    mat4 shadowMatrix;
};

struct cbMain
{
    vec3 qt_cameraPosition;
    vec4 qt_lightAndShadowCounts;
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
    mat4 qt_viewProjectionMatrix;
};

uniform cbMain _508;

struct cbDirectionalLights
{
    DirectionalLightData directionalLights[4];
};

uniform cbDirectionalLights ubDirectionalLights;

struct cbLights
{
    LightData lights[16];
};

uniform cbLights ubLights;

uniform highp samplerCube qt_lightProbe;
uniform highp sampler2D qt_BaseColorMap_sampler;
uniform highp sampler2D qt_screenTexture;
uniform highp sampler2D qt_shadowmap_blue_noise_texture;
uniform highp sampler2DArray qt_shadowmap_texture;

in vec3 qt_varWorldPos;
in vec3 qt_varNormal;
in vec3 qt_varTangent;
in vec3 qt_varBinormal;
in vec2 qt_varTexCoord0;
layout(location = 0) out vec4 fragOutput;

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

float qt_schlick(float value)
{
    float n = 1.0 - value;
    float n2 = n * n;
    return (n2 * n2) * n;
}

vec4 qt_diffuseBurleyBSDF(vec3 normal, vec3 lightDirection, vec3 viewVector, vec3 lightDiffuse, float roughness)
{
    vec3 H = normalize(viewVector + lightDirection);
    float cLdotH = max(0.0, dot(lightDirection, H));
    float cNdotV = max(0.0, dot(normal, viewVector));
    float cNdotL = max(0.0, dot(normal, lightDirection));
    float param = cNdotL;
    float fLight = qt_schlick(param);
    float param_1 = cNdotV;
    float fView = qt_schlick(param_1);
    float fDiffuse90 = 0.5 + (((2.0 * cLdotH) * cLdotH) * roughness);
    float fDiffuse = mix(1.0, fDiffuse90, fLight) * mix(1.0, fDiffuse90, fView);
    float factor = fDiffuse * cNdotL;
    return vec4(lightDiffuse * factor, 1.0);
}

float qt_D_GGX(float cos_theta_m, float alpha)
{
    float alpha2 = alpha * alpha;
    float d = 1.0 + (((alpha2 - 1.0) * cos_theta_m) * cos_theta_m);
    return alpha2 / ((3.1415927410125732 * d) * d);
}

float qt_G_GGX_2cos(float cos_theta_m, float alpha)
{
    float k = 0.5 * alpha;
    return 0.5 / ((cos_theta_m * (1.0 - k)) + k);
}

float qt_V_GGX(float NdotL, float NdotV, float alphaRoughness)
{
    float param = NdotL;
    float param_1 = alphaRoughness;
    float param_2 = NdotV;
    float param_3 = alphaRoughness;
    return qt_G_GGX_2cos(param, param_1) * qt_G_GGX_2cos(param_2, param_3);
}

vec4 qt_specularGGXBSDF(vec3 normal, vec3 L, vec3 V, vec3 lightSpecular, vec3 F0, vec3 F90, float roughness)
{
    float NdotL = dot(normal, L);
    float cNdotL = max(0.0, NdotL);
    float NdotV = dot(normal, V);
    float cNdotV = max(0.0, NdotV);
    vec3 H = normalize(V + L);
    float cLdotH = max(0.0, dot(L, H));
    float cNdotH = max(0.0010000000474974513, dot(normal, H));
    float alpha_ggx = clamp(roughness * roughness, 0.0010000000474974513, 1.0);
    float param = cNdotH;
    float param_1 = alpha_ggx;
    float GGX_D = qt_D_GGX(param, param_1);
    float param_2 = cNdotL;
    float param_3 = cNdotV;
    float param_4 = alpha_ggx;
    float GGX_V = qt_V_GGX(param_2, param_3, param_4);
    float param_5 = cLdotH;
    float cLdotH5 = qt_schlick(param_5);
    vec3 F = mix(vec3(cLdotH5), F90, F0);
    vec3 specular_brdf_NL = (F * (cNdotL * GGX_D)) * GGX_V;
    return vec4(specular_brdf_NL * lightSpecular, 1.0);
}

void qt_calculateSpecularContribution(inout vec3 global_specular_light, vec3 lightSpecularColor, float lightAttenuation, ShadowResult shadow, vec3 specularAmount, vec3 specularLightDirection, vec3 worldNormal, float roughnessAmount, float metalnessAmount, vec3 viewVector, vec3 f0, vec3 f90, vec3 specularTint)
{
    if (!shadow.isLit)
    {
        return;
    }
    vec3 param = worldNormal;
    vec3 param_1 = -specularLightDirection;
    vec3 param_2 = viewVector;
    vec3 param_3 = lightSpecularColor;
    vec3 param_4 = f0;
    vec3 param_5 = f90;
    float param_6 = roughnessAmount;
    global_specular_light += ((specularTint * (lightAttenuation * shadow.illumination)) * qt_specularGGXBSDF(param, param_1, param_2, param_3, param_4, param_5, param_6).xyz);
}

float qt_calculatePointLightAttenuation(vec3 attenuationConstants, float lightDistance)
{
    return 1.0 / ((attenuationConstants.x + (attenuationConstants.y * lightDistance)) + ((attenuationConstants.z * lightDistance) * lightDistance));
}

void qt_processPunctualLighting(inout vec3 global_diffuse_light, inout vec3 global_specular_light, vec3 qt_diffuseColor, vec3 qt_world_position, vec3 qt_world_normal, vec3 qt_view_vector, vec3 specularAmount, vec3 specularTint, float qt_roughnessAmount, float qt_metalnessAmount, vec3 f0, vec3 f90)
{
    ShadowResult qt_shadow_map_result;
    for (int i = 0; float(i) < _508.qt_lightAndShadowCounts.y; i++)
    {
        qt_shadow_map_result.illumination = 1.0;
        qt_shadow_map_result.isLit = true;
        vec3 tmp_light_color = ubDirectionalLights.directionalLights[i].diffuseColor * (1.0 - qt_metalnessAmount);
        vec3 param = qt_world_normal;
        vec3 param_1 = -ubDirectionalLights.directionalLights[i].direction;
        vec3 param_2 = qt_view_vector;
        vec3 param_3 = tmp_light_color;
        float param_4 = qt_roughnessAmount;
        global_diffuse_light += ((qt_diffuseColor * qt_shadow_map_result.illumination) * qt_diffuseBurleyBSDF(param, param_1, param_2, param_3, param_4).xyz);
        vec3 param_5 = global_specular_light;
        vec3 param_6 = ubDirectionalLights.directionalLights[i].specularColor;
        float param_7 = 1.0;
        ShadowResult param_8 = qt_shadow_map_result;
        vec3 param_9 = specularAmount;
        vec3 param_10 = ubDirectionalLights.directionalLights[i].direction;
        vec3 param_11 = qt_world_normal;
        float param_12 = qt_roughnessAmount;
        float param_13 = qt_metalnessAmount;
        vec3 param_14 = qt_view_vector;
        vec3 param_15 = f0;
        vec3 param_16 = f90;
        vec3 param_17 = specularTint;
        qt_calculateSpecularContribution(param_5, param_6, param_7, param_8, param_9, param_10, param_11, param_12, param_13, param_14, param_15, param_16, param_17);
        global_specular_light = param_5;
    }
    ShadowResult qt_shadow_map_result_1;
    for (int i_1 = 0; float(i_1) < _508.qt_lightAndShadowCounts.x; i_1++)
    {
        qt_shadow_map_result_1.illumination = 1.0;
        qt_shadow_map_result_1.isLit = true;
        vec3 relativeDirection = qt_world_position - ubLights.lights[i_1].position;
        float relativeDistance = length(relativeDirection);
        vec3 normalizedDirection = relativeDirection / vec3(relativeDistance);
        vec3 specularLightDirection = normalizedDirection;
        vec3 param_18 = vec3(ubLights.lights[i_1].constantAttenuation, ubLights.lights[i_1].linearAttenuation, ubLights.lights[i_1].quadraticAttenuation);
        float param_19 = relativeDistance;
        float qt_lightAttenuation = qt_calculatePointLightAttenuation(param_18, param_19);
        vec3 tmp_light_color_1 = ubLights.lights[i_1].diffuseColor * (1.0 - qt_metalnessAmount);
        bool isPointLight = ubLights.lights[i_1].coneAngle > 1.0;
        if (isPointLight)
        {
            vec3 param_20 = qt_world_normal;
            vec3 param_21 = -normalizedDirection;
            vec3 param_22 = qt_view_vector;
            vec3 param_23 = tmp_light_color_1;
            float param_24 = qt_roughnessAmount;
            global_diffuse_light += (((qt_diffuseColor * qt_lightAttenuation) * qt_shadow_map_result_1.illumination) * qt_diffuseBurleyBSDF(param_20, param_21, param_22, param_23, param_24).xyz);
        }
        else
        {
            float spotAngle = dot(normalizedDirection, normalize(ubLights.lights[i_1].direction));
            if (spotAngle > ubLights.lights[i_1].coneAngle)
            {
                float spotFactor = smoothstep(ubLights.lights[i_1].coneAngle, ubLights.lights[i_1].innerConeAngle, spotAngle);
                vec3 param_25 = qt_world_normal;
                vec3 param_26 = -normalizedDirection;
                vec3 param_27 = qt_view_vector;
                vec3 param_28 = tmp_light_color_1;
                float param_29 = qt_roughnessAmount;
                global_diffuse_light += ((((qt_diffuseColor * spotFactor) * qt_lightAttenuation) * qt_shadow_map_result_1.illumination) * qt_diffuseBurleyBSDF(param_25, param_26, param_27, param_28, param_29).xyz);
                qt_lightAttenuation *= spotFactor;
            }
            else
            {
                qt_lightAttenuation = 0.0;
            }
        }
        vec3 param_30 = global_specular_light;
        vec3 param_31 = ubLights.lights[i_1].specularColor;
        float param_32 = qt_lightAttenuation;
        ShadowResult param_33 = qt_shadow_map_result_1;
        vec3 param_34 = specularAmount;
        vec3 param_35 = specularLightDirection;
        vec3 param_36 = qt_world_normal;
        float param_37 = qt_roughnessAmount;
        float param_38 = qt_metalnessAmount;
        vec3 param_39 = qt_view_vector;
        vec3 param_40 = f0;
        vec3 param_41 = f90;
        vec3 param_42 = specularTint;
        qt_calculateSpecularContribution(param_30, param_31, param_32, param_33, param_34, param_35, param_36, param_37, param_38, param_39, param_40, param_41, param_42);
        global_specular_light = param_30;
    }
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
    if (_508.qt_lightProbeProperties.w < 0.004999999888241291)
    {
        return vec4(0.0);
    }
    vec3 smpDir = normal;
    float baseLevel = _508.qt_lightProbeProperties.y;
    vec3 param = smpDir;
    float param_1 = baseLevel;
    vec3 val = qt_textureProbe(qt_lightProbe, param, param_1);
    if (_508.qt_lightProbeProperties.z > (-1.0))
    {
        float ctr = 0.5 + (0.5 * _508.qt_lightProbeProperties.z);
        float vertWt = smoothstep(ctr * 0.25, ctr + 0.25, smpDir.y);
        float wtScaled = mix(1.0, vertWt, _508.qt_lightProbeProperties.z + 1.0);
        val *= wtScaled;
    }
    vec3 param_2 = val;
    float param_3 = _508.qt_lightProbeProperties.w;
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
    if (_508.qt_lightProbeProperties.w < 0.004999999888241291)
    {
        return vec4(0.0);
    }
    float levels = _508.qt_lightProbeProperties.y - 1.0;
    float lod = clamp(roughness * 5.0, 0.0, levels);
    vec3 smpDir = normalize(reflect(-viewDir, normal));
    vec3 param = smpDir;
    float param_1 = lod;
    vec3 specularSample = qt_textureProbe(qt_lightProbe, param, param_1);
    vec3 param_2 = normal;
    vec3 param_3 = viewDir;
    float param_4 = roughness;
    vec2 brdf = qt_brdfApproximation(param_2, param_3, param_4);
    if (_508.qt_lightProbeProperties.z > (-1.0))
    {
        float ctr = 0.5 + (0.5 * _508.qt_lightProbeProperties.z);
        float vertWt = smoothstep(ctr * 0.25, ctr + 0.25, smpDir.y);
        float wtScaled = mix(1.0, vertWt, _508.qt_lightProbeProperties.z + 1.0);
        specularSample *= wtScaled;
    }
    vec3 sampleColor = specularSample * ((F * brdf.x) + vec3(brdf.y));
    vec3 param_5 = sampleColor;
    float param_6 = _508.qt_lightProbeProperties.w;
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
    float qt_objectOpacity = _508.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec3 qt_view_vector = normalize(_508.qt_cameraPosition - qt_varWorldPos);
    vec3 qt_world_normal = normalize(qt_varNormal);
    vec3 qt_tangent = normalize(qt_varTangent);
    vec3 qt_binormal = normalize(qt_varBinormal);
    vec4 qt_diffuseColor = _508.qt_material_base_color * qt_vertColor;
    vec3 qt_global_emission = _508.qt_material_emissive_color;
    float qt_iOR = _508.qt_material_specular.w;
    vec3 qt_specularTint = _508.qt_material_specular.xyz;
    vec2 qt_texCoord0 = qt_varTexCoord0;
    vec4 qt_base_texture_color = texture(qt_BaseColorMap_sampler, qt_texCoord0);
    vec4 param = qt_base_texture_color;
    qt_base_texture_color = qt_sRGBToLinear(param);
    qt_diffuseColor *= qt_base_texture_color;
    float qt_aoFactor = 1.0;
    float qt_roughnessAmount = _508.qt_material_properties.y;
    float qt_metalnessAmount = _508.qt_material_properties.z;
    vec3 qt_specularBase = vec3(1.0);
    float qt_specularFactor = _508.qt_material_properties.x;
    vec4 global_diffuse_light = vec4(0.0);
    global_diffuse_light = vec4((_508.qt_light_ambient_total * (1.0 - qt_metalnessAmount)) * qt_diffuseColor.xyz, 0.0);
    vec3 global_specular_light = vec3(0.0);
    vec3 qt_specularAmount = qt_specularBase * vec3(qt_metalnessAmount + (qt_specularFactor * (1.0 - qt_metalnessAmount)));
    vec3 qt_f0 = vec3(1.0);
    vec3 qt_f90 = vec3(1.0);
    float param_1 = qt_iOR;
    float param_2 = qt_metalnessAmount;
    vec3 param_3 = qt_diffuseColor.xyz;
    qt_f0 = qt_F0_ior(param_1, param_2, param_3);
    float qt_fresnelPower = _508.qt_material_properties2.x;
    vec3 param_4 = qt_world_normal;
    vec3 param_5 = qt_view_vector;
    vec3 param_6 = qt_f0;
    float param_7 = qt_roughnessAmount;
    float param_8 = qt_fresnelPower;
    vec3 qt_principledMaterialFresnelValue = qt_principledMaterialFresnel(param_4, param_5, param_6, param_7, param_8);
    qt_specularAmount *= qt_principledMaterialFresnelValue;
    qt_specularTint = mix(vec3(1.0), qt_specularTint, vec3(1.0 - qt_metalnessAmount));
    vec3 param_9 = global_diffuse_light.xyz;
    vec3 param_10 = global_specular_light;
    vec3 param_11 = qt_diffuseColor.xyz;
    vec3 param_12 = qt_varWorldPos;
    vec3 param_13 = qt_world_normal;
    vec3 param_14 = qt_view_vector;
    vec3 param_15 = qt_specularAmount;
    vec3 param_16 = qt_specularTint;
    float param_17 = qt_roughnessAmount;
    float param_18 = qt_metalnessAmount;
    vec3 param_19 = qt_f0;
    vec3 param_20 = qt_f90;
    qt_processPunctualLighting(param_9, param_10, param_11, param_12, param_13, param_14, param_15, param_16, param_17, param_18, param_19, param_20);
    global_diffuse_light.x = param_9.x;
    global_diffuse_light.y = param_9.y;
    global_diffuse_light.z = param_9.z;
    global_specular_light = param_10;
    global_diffuse_light = vec4(global_diffuse_light.xyz * qt_aoFactor, qt_objectOpacity * qt_diffuseColor.w);
    vec3 param_21 = qt_world_normal;
    vec3 qt_iblDiffuse = (qt_diffuseColor.xyz * (vec3(1.0) - qt_specularAmount)) * qt_sampleDiffuse(param_21).xyz;
    vec3 param_22 = qt_world_normal;
    vec3 param_23 = qt_view_vector;
    vec3 param_24 = qt_specularAmount;
    float param_25 = qt_roughnessAmount;
    vec3 qt_iblSpecular = qt_specularTint * qt_sampleGlossyPrincipled(param_22, param_23, param_24, param_25).xyz;
    vec4 _1207 = global_diffuse_light;
    vec3 _1209 = _1207.xyz + (qt_iblDiffuse * qt_aoFactor);
    global_diffuse_light.x = _1209.x;
    global_diffuse_light.y = _1209.y;
    global_diffuse_light.z = _1209.z;
    global_specular_light += (qt_iblSpecular * qt_aoFactor);
    vec4 _1223 = global_diffuse_light;
    vec3 _1225 = _1223.xyz * (1.0 - qt_metalnessAmount);
    global_diffuse_light.x = _1225.x;
    global_diffuse_light.y = _1225.y;
    global_diffuse_light.z = _1225.z;
    vec4 qt_color_sum = vec4((global_diffuse_light.xyz + global_specular_light) + qt_global_emission, global_diffuse_light.w);
    vec4 param_26 = qt_color_sum;
    fragOutput = qt_tonemap(param_26);
}

