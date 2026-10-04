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

struct RayData
{
    vec3 rayViewStart;
    vec3 rayViewEnd;
    vec2 rayFragStart;
    vec2 rayFragEnd;
    vec2 rayFragCurr;
    float rayFragStartDepth;
    float rayFragEndDepth;
    float rayCoveredPart;
    int hit;
    vec3 objHitViewPos;
};

struct cbMain
{
    float depthBias;
    int marchSteps;
    vec4 materialColor;
    vec3 qt_cameraDirection;
    vec3 qt_cameraPosition;
    vec2 qt_cameraProperties;
    mat4 qt_inverseProjectionMatrix;
    vec4 qt_lightAndShadowCounts;
    vec4 qt_lightProbeProperties;
    vec3 qt_light_ambient_total;
    vec4 qt_material_base_color;
    vec3 qt_material_emissive_color;
    vec4 qt_material_properties;
    vec4 qt_material_properties2;
    vec4 qt_material_properties3;
    vec4 qt_material_specular;
    mat4 qt_modelMatrix;
    mat4 qt_modelViewProjection;
    mat3 qt_normalMatrix;
    mat4 qt_projectionMatrix;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
    mat4 qt_viewProjectionMatrix;
    float rayMaxDistance;
    int refinementSteps;
    float specular;
};

uniform cbMain _547;

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
uniform highp sampler2D qt_screenTexture;
uniform highp sampler2D qt_depthTexture;
uniform highp sampler2D qt_shadowmap_blue_noise_texture;
uniform highp sampler2DArray qt_shadowmap_texture;

in vec3 qt_varNormal;
in vec3 qt_varWorldPos;
in vec3 qt_varTangent;
in vec3 qt_varBinormal;
in vec2 qt_varTexCoord0;
in vec2 qt_varTexCoord1;
layout(location = 0) out vec4 fragOutput;
RayData rayData;

float rayViewDepthFromScreen(vec2 size)
{
    vec2 uv = rayData.rayFragCurr / size;
    float depth = mix(rayData.rayFragStartDepth, rayData.rayFragEndDepth, rayData.rayCoveredPart);
    vec3 ndc = vec3((uv * 2.0) - vec2(1.0), depth);
    vec4 viewPos = _547.qt_inverseProjectionMatrix * vec4(ndc, 1.0);
    viewPos /= vec4(viewPos.w);
    return viewPos.z;
}

vec2 correctTextureCoordinates(inout vec2 uv)
{
    bool _1500 = _547.qt_rhi_properties.x < 0.0;
    bool _1506;
    if (_1500)
    {
        _1506 = _547.qt_rhi_properties.y == 1.0;
    }
    else
    {
        _1506 = _1500;
    }
    if (_1506)
    {
        uv.y = 1.0 - uv.y;
    }
    return uv;
}

vec3 viewPosFromScreen(vec2 fragPos, vec2 size)
{
    vec2 uv = fragPos / size;
    vec2 param = uv;
    vec2 _1242 = correctTextureCoordinates(param);
    vec2 texuv = _1242;
    float depth = textureLod(qt_depthTexture, texuv, 0.0).x;
    if (_547.qt_rhi_properties.z < 0.0)
    {
        depth = (2.0 * depth) - 1.0;
    }
    vec3 ndc = vec3((uv * 2.0) - vec2(1.0), depth);
    vec4 viewPos = _547.qt_inverseProjectionMatrix * vec4(ndc, 1.0);
    viewPos /= vec4(viewPos.w);
    return viewPos.xyz;
}

void refinementStep(inout vec2 rayStepVector, vec2 size)
{
    for (int i = 0; i < _547.refinementSteps; i++)
    {
        rayData.rayCoveredPart = length(rayData.rayFragCurr - rayData.rayFragStart) / length(rayData.rayFragEnd - rayData.rayFragStart);
        rayData.rayCoveredPart = clamp(rayData.rayCoveredPart, 0.0, 1.0);
        vec2 param = size;
        float rayDepth = rayViewDepthFromScreen(param);
        vec2 param_1 = rayData.rayFragCurr;
        vec2 param_2 = size;
        rayData.objHitViewPos = viewPosFromScreen(param_1, param_2);
        float deltaDepth = rayDepth - rayData.objHitViewPos.z;
        rayStepVector *= 0.5;
        bool _1406 = deltaDepth > 0.0;
        bool _1413;
        if (_1406)
        {
            _1413 = deltaDepth < _547.depthBias;
        }
        else
        {
            _1413 = _1406;
        }
        if (_1413)
        {
            rayData.rayFragCurr -= rayStepVector;
        }
        else
        {
            rayData.rayFragCurr += rayStepVector;
        }
    }
}

void rayMarch(vec2 rayStepVector, vec2 size)
{
    for (int i = 0; i < _547.marchSteps; i++)
    {
        rayData.rayFragCurr += rayStepVector;
        rayData.rayCoveredPart = length(rayData.rayFragCurr - rayData.rayFragStart) / length(rayData.rayFragEnd - rayData.rayFragStart);
        rayData.rayCoveredPart = clamp(rayData.rayCoveredPart, 0.0, 1.0);
        vec2 param = size;
        float rayDepth = rayViewDepthFromScreen(param);
        vec2 param_1 = rayData.rayFragCurr;
        vec2 param_2 = size;
        rayData.objHitViewPos = viewPosFromScreen(param_1, param_2);
        float deltaDepth = rayDepth - rayData.objHitViewPos.z;
        bool _1479 = deltaDepth > 0.0;
        bool _1486;
        if (_1479)
        {
            _1486 = deltaDepth < _547.depthBias;
        }
        else
        {
            _1486 = _1479;
        }
        if (_1486)
        {
            rayData.hit = 1;
            vec2 param_3 = rayStepVector;
            vec2 param_4 = size;
            refinementStep(param_3, param_4);
            return;
        }
    }
}

bool rayFragOutOfBound(vec2 rayFrag, vec2 size)
{
    bool _1329 = rayFrag.x > size.x;
    bool _1338;
    if (!_1329)
    {
        _1338 = rayFrag.y > size.y;
    }
    else
    {
        _1338 = _1329;
    }
    if (_1338)
    {
        return true;
    }
    bool _1344 = rayFrag.x < 0.0;
    bool _1351;
    if (!_1344)
    {
        _1351 = rayFrag.y < 0.0;
    }
    else
    {
        _1351 = _1344;
    }
    if (_1351)
    {
        return true;
    }
    return false;
}

void qt_customMain(out vec4 BASE_COLOR, vec3 EMISSIVE_COLOR, float METALNESS, float ROUGHNESS, float SPECULAR_AMOUNT, float FRESNEL_POWER, vec3 NORMAL, vec3 TANGENT, vec3 BINORMAL, vec2 UV0, vec2 UV1, vec3 VIEW_VECTOR, float IOR, float OCCLUSION_AMOUNT)
{
    vec2 size = vec2(textureSize(qt_screenTexture, 0));
    vec3 unitPos = normalize(-VIEW_VECTOR);
    vec3 unitNormal = normalize(qt_varNormal);
    vec3 reflected = reflect(unitPos, unitNormal);
    reflected = (_547.qt_viewMatrix * vec4(reflected, 0.0)).xyz;
    reflected = normalize(reflected);
    vec3 fragViewPos = (_547.qt_viewMatrix * vec4(qt_varWorldPos, 1.0)).xyz;
    rayData.rayViewStart = fragViewPos;
    rayData.rayViewEnd = fragViewPos + (reflected * _547.rayMaxDistance);
    vec4 rayClipStart = _547.qt_projectionMatrix * vec4(rayData.rayViewStart, 1.0);
    rayClipStart /= vec4(rayClipStart.w);
    rayData.rayFragStart = (rayClipStart.xy * 0.5) + vec2(0.5);
    rayData.rayFragStart *= size;
    rayData.rayFragStartDepth = rayClipStart.z;
    vec4 rayClipEnd = _547.qt_projectionMatrix * vec4(rayData.rayViewEnd, 1.0);
    rayClipEnd /= vec4(rayClipEnd.w);
    rayData.rayFragEnd = (rayClipEnd.xy * 0.5) + vec2(0.5);
    rayData.rayFragEnd *= size;
    rayData.rayFragEndDepth = rayClipEnd.z;
    vec2 diff = rayData.rayFragEnd - rayData.rayFragStart;
    vec2 rayStepVector = diff / vec2(float(max(_547.marchSteps, 1)));
    rayData.rayFragCurr = rayData.rayFragStart;
    rayData.hit = 0;
    vec2 param = rayStepVector;
    vec2 param_1 = size;
    rayMarch(param, param_1);
    vec2 param_2 = rayData.rayFragCurr;
    vec2 param_3 = size;
    bool isOutOfBound = rayFragOutOfBound(param_2, param_3);
    if (isOutOfBound)
    {
        BASE_COLOR = _547.materialColor;
        return;
    }
    float visibility = float(rayData.hit);
    visibility *= (1.0 - max(dot(-normalize(fragViewPos), reflected), 0.0));
    visibility *= (1.0 - clamp(length(rayData.objHitViewPos - rayData.rayViewStart) / _547.rayMaxDistance, 0.0, 1.0));
    visibility = clamp(visibility, 0.0, 1.0);
    vec2 uv = rayData.rayFragCurr / size;
    vec2 param_4 = uv;
    vec2 _1209 = correctTextureCoordinates(param_4);
    uv = _1209;
    vec3 reflectionColor = texture(qt_screenTexture, uv).xyz;
    reflectionColor *= _547.specular;
    vec3 mixedColor = mix(_547.materialColor.xyz, reflectionColor, vec3(visibility));
    BASE_COLOR = vec4(mixedColor, _547.materialColor.w);
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
    for (int i = 0; float(i) < _547.qt_lightAndShadowCounts.y; i++)
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
    for (int i_1 = 0; float(i_1) < _547.qt_lightAndShadowCounts.x; i_1++)
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
    if (_547.qt_lightProbeProperties.w < 0.004999999888241291)
    {
        return vec4(0.0);
    }
    vec3 smpDir = normal;
    float baseLevel = _547.qt_lightProbeProperties.y;
    vec3 param = smpDir;
    float param_1 = baseLevel;
    vec3 val = qt_textureProbe(qt_lightProbe, param, param_1);
    if (_547.qt_lightProbeProperties.z > (-1.0))
    {
        float ctr = 0.5 + (0.5 * _547.qt_lightProbeProperties.z);
        float vertWt = smoothstep(ctr * 0.25, ctr + 0.25, smpDir.y);
        float wtScaled = mix(1.0, vertWt, _547.qt_lightProbeProperties.z + 1.0);
        val *= wtScaled;
    }
    vec3 param_2 = val;
    float param_3 = _547.qt_lightProbeProperties.w;
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
    if (_547.qt_lightProbeProperties.w < 0.004999999888241291)
    {
        return vec4(0.0);
    }
    float levels = _547.qt_lightProbeProperties.y - 1.0;
    float lod = clamp(roughness * 5.0, 0.0, levels);
    vec3 smpDir = normalize(reflect(-viewDir, normal));
    vec3 param = smpDir;
    float param_1 = lod;
    vec3 specularSample = qt_textureProbe(qt_lightProbe, param, param_1);
    vec3 param_2 = normal;
    vec3 param_3 = viewDir;
    float param_4 = roughness;
    vec2 brdf = qt_brdfApproximation(param_2, param_3, param_4);
    if (_547.qt_lightProbeProperties.z > (-1.0))
    {
        float ctr = 0.5 + (0.5 * _547.qt_lightProbeProperties.z);
        float vertWt = smoothstep(ctr * 0.25, ctr + 0.25, smpDir.y);
        float wtScaled = mix(1.0, vertWt, _547.qt_lightProbeProperties.z + 1.0);
        specularSample *= wtScaled;
    }
    vec3 sampleColor = specularSample * ((F * brdf.x) + vec3(brdf.y));
    vec3 param_5 = sampleColor;
    float param_6 = _547.qt_lightProbeProperties.w;
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
    float qt_objectOpacity = _547.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec3 qt_view_vector = normalize(_547.qt_cameraPosition - qt_varWorldPos);
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
    vec2 qt_texCoord1 = qt_varTexCoord1;
    vec4 param = qt_customBaseColor;
    vec3 param_1 = qt_customEmissiveColor;
    float param_2 = qt_customMetalnessAmount;
    float param_3 = qt_customSpecularRoughness;
    float param_4 = qt_customSpecularAmount;
    float param_5 = qt_customFresnelPower;
    vec3 param_6 = qt_world_normal;
    vec3 param_7 = qt_tangent;
    vec3 param_8 = qt_binormal;
    vec2 param_9 = qt_texCoord0;
    vec2 param_10 = qt_texCoord1;
    vec3 param_11 = qt_view_vector;
    float param_12 = qt_customIOR;
    float param_13 = qt_customOcclusionAmount;
    qt_customMain(param, param_1, param_2, param_3, param_4, param_5, param_6, param_7, param_8, param_9, param_10, param_11, param_12, param_13);
    qt_customBaseColor = param;
    qt_customEmissiveColor = param_1;
    qt_customMetalnessAmount = param_2;
    qt_customSpecularRoughness = param_3;
    qt_customSpecularAmount = param_4;
    qt_customFresnelPower = param_5;
    qt_world_normal = param_6;
    qt_tangent = param_7;
    qt_binormal = param_8;
    qt_customIOR = param_12;
    qt_customOcclusionAmount = param_13;
    vec4 qt_diffuseColor = qt_customBaseColor * qt_vertColor;
    vec3 qt_global_emission = qt_customEmissiveColor;
    float qt_iOR = qt_customIOR;
    vec3 qt_specularTint = vec3(1.0);
    float qt_aoFactor = 1.0;
    qt_aoFactor *= qt_customOcclusionAmount;
    float qt_roughnessAmount = qt_customSpecularRoughness;
    float qt_metalnessAmount = qt_customMetalnessAmount;
    vec3 qt_specularBase = vec3(1.0);
    float qt_specularFactor = qt_customSpecularAmount;
    vec4 global_diffuse_light = vec4(0.0);
    global_diffuse_light = vec4((_547.qt_light_ambient_total * (1.0 - qt_metalnessAmount)) * qt_diffuseColor.xyz, 0.0);
    vec3 global_specular_light = vec3(0.0);
    vec3 qt_specularAmount = qt_specularBase * vec3(qt_metalnessAmount + (qt_specularFactor * (1.0 - qt_metalnessAmount)));
    vec3 qt_f0 = vec3(1.0);
    vec3 qt_f90 = vec3(1.0);
    float param_14 = qt_iOR;
    float param_15 = qt_metalnessAmount;
    vec3 param_16 = qt_diffuseColor.xyz;
    qt_f0 = qt_F0_ior(param_14, param_15, param_16);
    float qt_fresnelPower = qt_customFresnelPower;
    vec3 param_17 = qt_world_normal;
    vec3 param_18 = qt_view_vector;
    vec3 param_19 = qt_f0;
    float param_20 = qt_roughnessAmount;
    float param_21 = qt_fresnelPower;
    vec3 qt_principledMaterialFresnelValue = qt_principledMaterialFresnel(param_17, param_18, param_19, param_20, param_21);
    qt_specularAmount *= qt_principledMaterialFresnelValue;
    qt_specularTint = mix(vec3(1.0), qt_specularTint, vec3(1.0 - qt_metalnessAmount));
    vec3 param_22 = global_diffuse_light.xyz;
    vec3 param_23 = global_specular_light;
    vec3 param_24 = qt_diffuseColor.xyz;
    vec3 param_25 = qt_varWorldPos;
    vec3 param_26 = qt_world_normal;
    vec3 param_27 = qt_view_vector;
    vec3 param_28 = qt_specularAmount;
    vec3 param_29 = qt_specularTint;
    float param_30 = qt_roughnessAmount;
    float param_31 = qt_metalnessAmount;
    vec3 param_32 = qt_f0;
    vec3 param_33 = qt_f90;
    qt_processPunctualLighting(param_22, param_23, param_24, param_25, param_26, param_27, param_28, param_29, param_30, param_31, param_32, param_33);
    global_diffuse_light.x = param_22.x;
    global_diffuse_light.y = param_22.y;
    global_diffuse_light.z = param_22.z;
    global_specular_light = param_23;
    global_diffuse_light = vec4(global_diffuse_light.xyz * qt_aoFactor, qt_objectOpacity * qt_diffuseColor.w);
    vec3 param_34 = qt_world_normal;
    vec3 qt_iblDiffuse = (qt_diffuseColor.xyz * (vec3(1.0) - qt_specularAmount)) * qt_sampleDiffuse(param_34).xyz;
    vec3 param_35 = qt_world_normal;
    vec3 param_36 = qt_view_vector;
    vec3 param_37 = qt_specularAmount;
    float param_38 = qt_roughnessAmount;
    vec3 qt_iblSpecular = qt_specularTint * qt_sampleGlossyPrincipled(param_35, param_36, param_37, param_38).xyz;
    vec4 _1748 = global_diffuse_light;
    vec3 _1750 = _1748.xyz + (qt_iblDiffuse * qt_aoFactor);
    global_diffuse_light.x = _1750.x;
    global_diffuse_light.y = _1750.y;
    global_diffuse_light.z = _1750.z;
    global_specular_light += (qt_iblSpecular * qt_aoFactor);
    vec4 _1764 = global_diffuse_light;
    vec3 _1766 = _1764.xyz * (1.0 - qt_metalnessAmount);
    global_diffuse_light.x = _1766.x;
    global_diffuse_light.y = _1766.y;
    global_diffuse_light.z = _1766.z;
    vec4 qt_color_sum = vec4((global_diffuse_light.xyz + global_specular_light) + qt_global_emission, global_diffuse_light.w);
    vec4 param_39 = qt_color_sum;
    fragOutput = qt_tonemap(param_39);
}

