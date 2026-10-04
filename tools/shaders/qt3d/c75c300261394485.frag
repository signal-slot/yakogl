#version 300 es
precision highp float;
precision highp int;

struct ShadowResult
{
    float illumination;
    bool isLit;
};

const vec2 _495[4] = vec2[](vec2(0.23640461266040802, -0.7455790638923645), vec2(-0.5245236754417419, 0.8360051512718201), vec2(-0.8303866982460022, -0.22100143134593964), vec2(0.8316259384155273, 0.39830875396728516));

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

const vec3 _978[4] = vec3[](vec3(-0.4877236783504486, 0.4467948377132416, -0.75), vec3(0.08464959263801575, -0.9645384550094604, -0.25), vec3(0.5891184210777283, 0.7684006094932556, 0.25), vec3(0.6614378094673157, 0.0, 0.75));

struct cbMain
{
    vec3 qt_cameraPosition;
    vec4 qt_lightAndShadowCounts;
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

uniform cbMain _413;

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

uniform highp sampler2D qt_shadowmap_blue_noise_texture;
uniform highp sampler2DArray qt_shadowmap_texture;
uniform highp sampler2D qt_NormalMap_sampler;
uniform highp sampler2D qt_BaseColorMap_sampler;
uniform highp sampler2D qt_screenTexture;

in vec3 qt_varWorldPos;
in vec3 qt_varNormal;
in vec3 qt_varTangent;
in vec3 qt_varBinormal;
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

int findSplitIndex(float depth, vec4 splits, vec4 splitActive, uint numSplits)
{
    vec4 depthVec = vec4(depth);
    vec4 _lessThan = step(depthVec, splits);
    vec4 valid = _lessThan * splitActive;
    float result = -1.0;
    float notFoundYet = 1.0;
    result = mix(result, 0.0, valid.x * notFoundYet);
    notFoundYet *= (1.0 - valid.x);
    result = mix(result, 1.0, valid.y * notFoundYet);
    notFoundYet *= (1.0 - valid.y);
    result = mix(result, 2.0, valid.z * notFoundYet);
    notFoundYet *= (1.0 - valid.z);
    result = mix(result, 3.0, valid.w * notFoundYet);
    return int(result);
}

mat2 randomRotation2D(vec2 pixelPos)
{
    vec2 blueNoiseUV = pixelPos / vec2(64.0);
    vec4 noiseSample = texture(qt_shadowmap_blue_noise_texture, blueNoiseUV);
    float cosTheta = (noiseSample.x * 2.0) - 1.0;
    float sinTheta = (noiseSample.y * 2.0) - 1.0;
    return mat2(vec2(cosTheta, -sinTheta), vec2(sinTheta, cosTheta));
}

ShadowResult normalizedShadowResult(float illumination, float factor)
{
    float normalizedFactor = clamp(factor, 0.0, 100.0) * 0.009999999776482582;
    ShadowResult result;
    result.illumination = min(1.0, 1.0 - (normalizedFactor * (1.0 - illumination)));
    result.isLit = illumination > 0.0;
    return result;
}

ShadowResult qt_sampleDirectionalLight_splitIndex(int splitIndex, float shadowBias, float shadowFactor, vec3 dimensionsInverted, float pcfFactor, uint numSamples, vec4 projCoord, vec3 worldPos, float layerIndex, vec2 uvAtlasOffset, float uvAtlasScale)
{
    vec3 smpCoord = projCoord.xyz / vec3(projCoord.w);
    float flipY = step(_413.qt_rhi_properties.x, -0.5);
    smpCoord.y = mix(smpCoord.y, 1.0 - smpCoord.y, flipY);
    float inBounds = ((((step(0.0, smpCoord.x) * step(smpCoord.x, 1.0)) * step(0.0, smpCoord.y)) * step(smpCoord.y, 1.0)) * step(0.0, smpCoord.z)) * step(smpCoord.z, 1.0);
    float illumination = 0.0;
    vec2 radiusUV = dimensionsInverted.xy * pcfFactor;
    float bias = shadowBias * dimensionsInverted.z;
    vec2 param = gl_FragCoord.xy;
    mat2 randomRotation = randomRotation2D(param);
    for (uint i = 0u; i < 4u; i++)
    {
        vec2 sampleOffset = randomRotation * (_495[i] * radiusUV);
        vec3 atlasCoord = vec3(((smpCoord.xy + sampleOffset) * uvAtlasScale) + uvAtlasOffset, layerIndex);
        float pcfDepth = textureLod(qt_shadowmap_texture, atlasCoord, 0.0).x + bias;
        illumination += step(smpCoord.z, pcfDepth);
    }
    illumination /= 4.0;
    illumination = mix(1.0, illumination, inBounds);
    float param_1 = illumination;
    float param_2 = shadowFactor;
    return normalizedShadowResult(param_1, param_2);
}

ShadowResult qt_sampleDirectionalLight_pcf(int lightIndex, float zDepthViewSpace, vec3 worldPos)
{
    uint numSamples = ubDirectionalLights.directionalLights[lightIndex].pcfSamples;
    float param = zDepthViewSpace;
    vec4 param_1 = ubDirectionalLights.directionalLights[lightIndex].csmSplits;
    vec4 param_2 = ubDirectionalLights.directionalLights[lightIndex].csmActive;
    uint param_3 = ubDirectionalLights.directionalLights[lightIndex].csmNumSplits;
    int splitIndex = findSplitIndex(param, param_1, param_2, param_3);
    float validSplit = step(0.0, float(splitIndex));
    vec4 projCoord = ubDirectionalLights.directionalLights[lightIndex].matrices[splitIndex] * vec4(worldPos, 1.0);
    vec3 dimensions = ubDirectionalLights.directionalLights[lightIndex].dimensionsInverted[splitIndex].xyz;
    vec4 shadowAtlasInfo = ubDirectionalLights.directionalLights[lightIndex].atlasLocations[splitIndex];
    float layerIndex = shadowAtlasInfo.w;
    vec2 uvAtlasOffset = shadowAtlasInfo.xy;
    float uvAtlasScale = shadowAtlasInfo.z;
    int param_4 = splitIndex;
    float param_5 = ubDirectionalLights.directionalLights[lightIndex].shadowBias;
    float param_6 = ubDirectionalLights.directionalLights[lightIndex].shadowFactor;
    vec3 param_7 = dimensions;
    float param_8 = ubDirectionalLights.directionalLights[lightIndex].pcfFactor;
    uint param_9 = numSamples;
    vec4 param_10 = projCoord;
    vec3 param_11 = worldPos;
    float param_12 = layerIndex;
    vec2 param_13 = uvAtlasOffset;
    float param_14 = uvAtlasScale;
    ShadowResult shadow = qt_sampleDirectionalLight_splitIndex(param_4, param_5, param_6, param_7, param_8, param_9, param_10, param_11, param_12, param_13, param_14);
    float shouldBlend = step(1.0, float(splitIndex));
    float splitPrev = ubDirectionalLights.directionalLights[lightIndex].csmSplits[max((splitIndex - 1), 0)];
    float bandLength = ubDirectionalLights.directionalLights[lightIndex].shadowMapFar * ubDirectionalLights.directionalLights[lightIndex].csmBlendRatio;
    float inBlendRegion = step(zDepthViewSpace, splitPrev + bandLength);
    float prevSplitActive = ubDirectionalLights.directionalLights[lightIndex].csmActive[max((splitIndex - 1), 0)];
    float doBlend = (shouldBlend * inBlendRegion) * prevSplitActive;
    vec4 shadowAtlasInfoPrev = ubDirectionalLights.directionalLights[lightIndex].atlasLocations[max((splitIndex - 1), 0)];
    float layerIndexPrev = shadowAtlasInfoPrev.w;
    vec2 uvAtlasOffsetPrev = shadowAtlasInfoPrev.xy;
    float uvAtlasScalePrev = shadowAtlasInfoPrev.z;
    vec4 projCoordPrev = ubDirectionalLights.directionalLights[lightIndex].matrices[max((splitIndex - 1), 0)] * vec4(worldPos, 1.0);
    vec3 dimensionsPrev = ubDirectionalLights.directionalLights[lightIndex].dimensionsInverted[max((splitIndex - 1), 0)].xyz;
    int param_15 = max((splitIndex - 1), 0);
    float param_16 = ubDirectionalLights.directionalLights[lightIndex].shadowBias;
    float param_17 = ubDirectionalLights.directionalLights[lightIndex].shadowFactor;
    vec3 param_18 = dimensionsPrev;
    float param_19 = ubDirectionalLights.directionalLights[lightIndex].pcfFactor;
    uint param_20 = numSamples;
    vec4 param_21 = projCoordPrev;
    vec3 param_22 = worldPos;
    float param_23 = layerIndexPrev;
    vec2 param_24 = uvAtlasOffsetPrev;
    float param_25 = uvAtlasScalePrev;
    ShadowResult shadowPrev = qt_sampleDirectionalLight_splitIndex(param_15, param_16, param_17, param_18, param_19, param_20, param_21, param_22, param_23, param_24, param_25);
    float t = clamp(((splitPrev + bandLength) - zDepthViewSpace) / max(bandLength, 9.999999974752427e-07), 0.0, 1.0);
    float blendedIllumination = mix(shadow.illumination, shadowPrev.illumination, t);
    shadow.illumination = mix(shadow.illumination, blendedIllumination, doBlend);
    shadow.illumination = mix(1.0, shadow.illumination, validSplit);
    if (validSplit < 0.5)
    {
        shadow.isLit = true;
    }
    return shadow;
}

ShadowResult qt_sampleDirectionalShadowMap(int lightIndex, float zDepthViewSpace, vec3 worldPos)
{
    int param = lightIndex;
    float param_1 = zDepthViewSpace;
    vec3 param_2 = worldPos;
    return qt_sampleDirectionalLight_pcf(param, param_1, param_2);
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

mat3 randomRotation3D(vec2 pixelPos)
{
    vec2 blueNoiseUV = pixelPos / vec2(64.0);
    vec4 noiseSample = texture(qt_shadowmap_blue_noise_texture, blueNoiseUV);
    float cosTheta = (noiseSample.x * 2.0) - 1.0;
    float sinTheta = (noiseSample.y * 2.0) - 1.0;
    float cosPhi = (noiseSample.z * 2.0) - 1.0;
    float sinPhi = (noiseSample.w * 2.0) - 1.0;
    return mat3(vec3(vec3(cosTheta * cosPhi, -sinTheta, cosTheta * sinPhi)), vec3(vec3(sinTheta * cosPhi, cosTheta, sinTheta * sinPhi)), vec3(vec3(-sinPhi, 0.0, cosPhi)));
}

ShadowResult qt_samplePointLight(int lightIndex, vec3 worldPos)
{
    vec3 lightPosition = ubLights.lights[lightIndex].position;
    vec3 L = worldPos - lightPosition;
    float dist = length(L);
    float nearZ = ubLights.lights[lightIndex].clipNear;
    float farZ = ubLights.lights[lightIndex].shadowMapFar;
    float mapSizeInv = 1.0 / (farZ - nearZ);
    float bias = ubLights.lights[lightIndex].shadowBias * mapSizeInv;
    uint numSamples = ubLights.lights[lightIndex].pcfSamples;
    float uvAtlasScale = ubLights.lights[lightIndex].shadowTextureSize;
    vec3 _861;
    if (dist > 0.0)
    {
        _861 = L / vec3(dist);
    }
    else
    {
        _861 = vec3(0.0, 0.0, 1.0);
    }
    vec3 d = _861;
    float forward = step(0.0, d.z);
    float denom = mix(1.0 - d.z, 1.0 + d.z, forward);
    denom = max(denom, 9.99999993922529e-09);
    float u = 0.5 * ((d.x / denom) + 1.0);
    float v = 0.5 * ((d.y / denom) + 1.0);
    float flipY = step(_413.qt_rhi_properties.y, 0.5);
    v = mix(v, 1.0 - v, flipY);
    float layer = mix(ubLights.lights[lightIndex].shadowAtlasLayer1, ubLights.lights[lightIndex].shadowAtlasLayer0, forward);
    vec2 uvAtlasOffset = mix(ubLights.lights[lightIndex].shadowAtlasUV1, ubLights.lights[lightIndex].shadowAtlasUV0, vec2(forward));
    float currentDepth = clamp((dist - nearZ) / (farZ - nearZ), 0.0, 1.0) - bias;
    float pcfRadius = ubLights.lights[lightIndex].pcfFactor;
    float shadowAccumulator = 0.0;
    vec2 param = gl_FragCoord.xy;
    mat3 randomRotation = randomRotation3D(param);
    vec3 _1001;
    for (uint i = 0u; i < 4u; i++)
    {
        vec3 offsetDir = (randomRotation * _978[i]) * pcfRadius;
        vec3 worldPosPCF = worldPos + offsetDir;
        vec3 Lpcf = worldPosPCF - lightPosition;
        float distPCF = length(Lpcf);
        if (distPCF > 0.0)
        {
            _1001 = Lpcf / vec3(distPCF);
        }
        else
        {
            _1001 = vec3(0.0, 0.0, 1.0);
        }
        vec3 dPCF = _1001;
        float forwardPCF = step(0.0, dPCF.z);
        float layerPCF = mix(ubLights.lights[lightIndex].shadowAtlasLayer1, ubLights.lights[lightIndex].shadowAtlasLayer0, forwardPCF);
        vec2 uvOffPCF = mix(ubLights.lights[lightIndex].shadowAtlasUV1, ubLights.lights[lightIndex].shadowAtlasUV0, vec2(forwardPCF));
        float denomPCF = mix(1.0 - dPCF.z, 1.0 + dPCF.z, forwardPCF);
        denomPCF = max(denomPCF, 9.99999993922529e-09);
        float uPCF = 0.5 * ((dPCF.x / denomPCF) + 1.0);
        float vPCF = 0.5 * ((dPCF.y / denomPCF) + 1.0);
        vPCF = mix(vPCF, 1.0 - vPCF, flipY);
        vec2 uvPCF = (vec2(uPCF, vPCF) * uvAtlasScale) + uvOffPCF;
        float storedDepth = textureLod(qt_shadowmap_texture, vec3(uvPCF, layerPCF), 0.0).x;
        shadowAccumulator += step(currentDepth, storedDepth);
    }
    float illumination = shadowAccumulator / 4.0;
    float param_1 = illumination;
    float param_2 = ubLights.lights[lightIndex].shadowFactor;
    return normalizedShadowResult(param_1, param_2);
}

ShadowResult fullyVisible()
{
    ShadowResult result;
    result.illumination = 1.0;
    result.isLit = true;
    return result;
}

ShadowResult qt_sampleSpotLight(int lightIndex, vec3 worldPos)
{
    vec3 dir = worldPos - ubLights.lights[lightIndex].position;
    vec3 Ldir = normalize(dir);
    float lightAngle = dot(Ldir, normalize(ubLights.lights[lightIndex].direction));
    if (lightAngle < ubLights.lights[lightIndex].coneAngle)
    {
        return fullyVisible();
    }
    float dist = length(dir);
    float nearZ = ubLights.lights[lightIndex].clipNear;
    float farZ = ubLights.lights[lightIndex].shadowMapFar;
    float mapSizeInv = 1.0 / (farZ - nearZ);
    float bias = ubLights.lights[lightIndex].shadowBias * mapSizeInv;
    float layerIndex = ubLights.lights[lightIndex].shadowAtlasLayer0;
    vec2 uvAtlasOffset = ubLights.lights[lightIndex].shadowAtlasUV0;
    float uvAtlasScale = ubLights.lights[lightIndex].shadowTextureSize;
    uint numSamples = ubLights.lights[lightIndex].pcfSamples;
    vec4 projCoord = ubLights.lights[lightIndex].shadowMatrix * vec4(worldPos, 1.0);
    vec3 smpCoord = projCoord.xyz / vec3(projCoord.w);
    float flipY = step(_413.qt_rhi_properties.x, -0.5);
    smpCoord.y = mix(smpCoord.y, 1.0 - smpCoord.y, flipY);
    float currentDepth = clamp((dist - nearZ) / (farZ - nearZ), 0.0, 1.0) - bias;
    float pcfRadius = ubLights.lights[lightIndex].pcfFactor;
    float accum = 0.0;
    vec2 param = gl_FragCoord.xy;
    mat3 randomRotation = randomRotation3D(param);
    for (uint i = 0u; i < 4u; i++)
    {
        vec3 offsetDir = (randomRotation * _978[i]) * pcfRadius;
        vec3 worldPosPCF = worldPos + offsetDir;
        vec4 projPCF = ubLights.lights[lightIndex].shadowMatrix * vec4(worldPosPCF, 1.0);
        vec3 smpPCF = projPCF.xyz / vec3(projPCF.w);
        smpPCF.y = mix(smpPCF.y, 1.0 - smpPCF.y, flipY);
        vec2 uvPCF = (smpPCF.xy * uvAtlasScale) + uvAtlasOffset;
        float storedDepth = textureLod(qt_shadowmap_texture, vec3(uvPCF, layerIndex), 0.0).x;
        accum += step(currentDepth, storedDepth);
    }
    float illumination = accum / 4.0;
    float param_1 = illumination;
    float param_2 = ubLights.lights[lightIndex].shadowFactor;
    return normalizedShadowResult(param_1, param_2);
}

void qt_processPunctualLighting(inout vec3 global_diffuse_light, inout vec3 global_specular_light, vec3 qt_diffuseColor, vec3 qt_world_position, vec3 qt_world_normal, vec3 qt_view_vector, vec3 specularAmount, vec3 specularTint, float qt_roughnessAmount, float qt_metalnessAmount, vec3 f0, vec3 f90)
{
    float qt_zDepthViewSpace = 0.0;
    if (_413.qt_lightAndShadowCounts.y > 0.0)
    {
        qt_zDepthViewSpace = abs((_413.qt_viewMatrix * vec4(qt_world_position, 1.0)).z);
    }
    ShadowResult qt_shadow_map_result;
    for (int i = 0; float(i) < _413.qt_lightAndShadowCounts.y; i++)
    {
        qt_shadow_map_result.illumination = 1.0;
        qt_shadow_map_result.isLit = true;
        if (ubDirectionalLights.directionalLights[i].enableShadows > 0.0)
        {
            int param = i;
            float param_1 = qt_zDepthViewSpace;
            vec3 param_2 = qt_world_position;
            qt_shadow_map_result = qt_sampleDirectionalShadowMap(param, param_1, param_2);
        }
        vec3 tmp_light_color = ubDirectionalLights.directionalLights[i].diffuseColor * (1.0 - qt_metalnessAmount);
        vec3 param_3 = qt_world_normal;
        vec3 param_4 = -ubDirectionalLights.directionalLights[i].direction;
        vec3 param_5 = qt_view_vector;
        vec3 param_6 = tmp_light_color;
        float param_7 = qt_roughnessAmount;
        global_diffuse_light += ((qt_diffuseColor * qt_shadow_map_result.illumination) * qt_diffuseBurleyBSDF(param_3, param_4, param_5, param_6, param_7).xyz);
        vec3 param_8 = global_specular_light;
        vec3 param_9 = ubDirectionalLights.directionalLights[i].specularColor;
        float param_10 = 1.0;
        ShadowResult param_11 = qt_shadow_map_result;
        vec3 param_12 = specularAmount;
        vec3 param_13 = ubDirectionalLights.directionalLights[i].direction;
        vec3 param_14 = qt_world_normal;
        float param_15 = qt_roughnessAmount;
        float param_16 = qt_metalnessAmount;
        vec3 param_17 = qt_view_vector;
        vec3 param_18 = f0;
        vec3 param_19 = f90;
        vec3 param_20 = specularTint;
        qt_calculateSpecularContribution(param_8, param_9, param_10, param_11, param_12, param_13, param_14, param_15, param_16, param_17, param_18, param_19, param_20);
        global_specular_light = param_8;
    }
    ShadowResult qt_shadow_map_result_1;
    for (int i_1 = 0; float(i_1) < _413.qt_lightAndShadowCounts.x; i_1++)
    {
        qt_shadow_map_result_1.illumination = 1.0;
        qt_shadow_map_result_1.isLit = true;
        vec3 relativeDirection = qt_world_position - ubLights.lights[i_1].position;
        float relativeDistance = length(relativeDirection);
        vec3 normalizedDirection = relativeDirection / vec3(relativeDistance);
        vec3 specularLightDirection = normalizedDirection;
        vec3 param_21 = vec3(ubLights.lights[i_1].constantAttenuation, ubLights.lights[i_1].linearAttenuation, ubLights.lights[i_1].quadraticAttenuation);
        float param_22 = relativeDistance;
        float qt_lightAttenuation = qt_calculatePointLightAttenuation(param_21, param_22);
        vec3 tmp_light_color_1 = ubLights.lights[i_1].diffuseColor * (1.0 - qt_metalnessAmount);
        bool isPointLight = ubLights.lights[i_1].coneAngle > 1.0;
        if (isPointLight)
        {
            if (ubLights.lights[i_1].enableShadows > 0.0)
            {
                int param_23 = i_1;
                vec3 param_24 = qt_world_position;
                qt_shadow_map_result_1 = qt_samplePointLight(param_23, param_24);
            }
            vec3 param_25 = qt_world_normal;
            vec3 param_26 = -normalizedDirection;
            vec3 param_27 = qt_view_vector;
            vec3 param_28 = tmp_light_color_1;
            float param_29 = qt_roughnessAmount;
            global_diffuse_light += (((qt_diffuseColor * qt_lightAttenuation) * qt_shadow_map_result_1.illumination) * qt_diffuseBurleyBSDF(param_25, param_26, param_27, param_28, param_29).xyz);
        }
        else
        {
            if (ubLights.lights[i_1].enableShadows > 0.0)
            {
                int param_30 = i_1;
                vec3 param_31 = qt_world_position;
                qt_shadow_map_result_1 = qt_sampleSpotLight(param_30, param_31);
            }
            float spotAngle = dot(normalizedDirection, normalize(ubLights.lights[i_1].direction));
            if (spotAngle > ubLights.lights[i_1].coneAngle)
            {
                float spotFactor = smoothstep(ubLights.lights[i_1].coneAngle, ubLights.lights[i_1].innerConeAngle, spotAngle);
                vec3 param_32 = qt_world_normal;
                vec3 param_33 = -normalizedDirection;
                vec3 param_34 = qt_view_vector;
                vec3 param_35 = tmp_light_color_1;
                float param_36 = qt_roughnessAmount;
                global_diffuse_light += ((((qt_diffuseColor * spotFactor) * qt_lightAttenuation) * qt_shadow_map_result_1.illumination) * qt_diffuseBurleyBSDF(param_32, param_33, param_34, param_35, param_36).xyz);
                qt_lightAttenuation *= spotFactor;
            }
            else
            {
                qt_lightAttenuation = 0.0;
            }
        }
        vec3 param_37 = global_specular_light;
        vec3 param_38 = ubLights.lights[i_1].specularColor;
        float param_39 = qt_lightAttenuation;
        ShadowResult param_40 = qt_shadow_map_result_1;
        vec3 param_41 = specularAmount;
        vec3 param_42 = specularLightDirection;
        vec3 param_43 = qt_world_normal;
        float param_44 = qt_roughnessAmount;
        float param_45 = qt_metalnessAmount;
        vec3 param_46 = qt_view_vector;
        vec3 param_47 = f0;
        vec3 param_48 = f90;
        vec3 param_49 = specularTint;
        qt_calculateSpecularContribution(param_37, param_38, param_39, param_40, param_41, param_42, param_43, param_44, param_45, param_46, param_47, param_48, param_49);
        global_specular_light = param_37;
    }
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
    float qt_objectOpacity = _413.qt_material_properties.w;
    vec4 qt_vertColorMask = vec4(1.0);
    vec4 qt_vertColor = vec4(1.0);
    vec3 qt_view_vector = normalize(_413.qt_cameraPosition - qt_varWorldPos);
    vec3 qt_world_normal = normalize(qt_varNormal);
    vec3 qt_tangent = normalize(qt_varTangent);
    vec3 qt_binormal = normalize(qt_varBinormal);
    vec4 qt_diffuseColor = _413.qt_material_base_color * qt_vertColor;
    vec3 qt_global_emission = _413.qt_material_emissive_color;
    float qt_iOR = _413.qt_material_specular.w;
    vec2 qt_texCoord0 = qt_varTexCoord0;
    float qt_normalStrength = _413.qt_material_properties2.y;
    float param = qt_normalStrength;
    vec2 param_1 = qt_texCoord0;
    vec3 param_2 = qt_tangent;
    vec3 param_3 = qt_binormal;
    vec3 param_4 = qt_world_normal;
    qt_world_normal = qt_sampleNormalTexture(qt_NormalMap_sampler, param, param_1, param_2, param_3, param_4);
    vec3 qt_specularTint = _413.qt_material_specular.xyz;
    vec4 qt_base_texture_color = texture(qt_BaseColorMap_sampler, qt_texCoord0);
    vec4 param_5 = qt_base_texture_color;
    qt_base_texture_color = qt_sRGBToLinear(param_5);
    qt_diffuseColor *= qt_base_texture_color;
    float qt_aoFactor = 1.0;
    float qt_roughnessAmount = _413.qt_material_properties.y;
    float qt_metalnessAmount = _413.qt_material_properties.z;
    vec3 qt_specularBase = vec3(1.0);
    float qt_specularFactor = _413.qt_material_properties.x;
    vec4 global_diffuse_light = vec4(0.0);
    global_diffuse_light = vec4((_413.qt_light_ambient_total * (1.0 - qt_metalnessAmount)) * qt_diffuseColor.xyz, 0.0);
    vec3 global_specular_light = vec3(0.0);
    vec3 qt_specularAmount = qt_specularBase * vec3(qt_metalnessAmount + (qt_specularFactor * (1.0 - qt_metalnessAmount)));
    vec3 qt_f0 = vec3(1.0);
    vec3 qt_f90 = vec3(1.0);
    float param_6 = qt_iOR;
    float param_7 = qt_metalnessAmount;
    vec3 param_8 = qt_diffuseColor.xyz;
    qt_f0 = qt_F0_ior(param_6, param_7, param_8);
    float qt_fresnelPower = _413.qt_material_properties2.x;
    vec3 param_9 = qt_world_normal;
    vec3 param_10 = qt_view_vector;
    vec3 param_11 = qt_f0;
    float param_12 = qt_roughnessAmount;
    float param_13 = qt_fresnelPower;
    vec3 qt_principledMaterialFresnelValue = qt_principledMaterialFresnel(param_9, param_10, param_11, param_12, param_13);
    qt_specularAmount *= qt_principledMaterialFresnelValue;
    qt_specularTint = mix(vec3(1.0), qt_specularTint, vec3(1.0 - qt_metalnessAmount));
    vec3 param_14 = global_diffuse_light.xyz;
    vec3 param_15 = global_specular_light;
    vec3 param_16 = qt_diffuseColor.xyz;
    vec3 param_17 = qt_varWorldPos;
    vec3 param_18 = qt_world_normal;
    vec3 param_19 = qt_view_vector;
    vec3 param_20 = qt_specularAmount;
    vec3 param_21 = qt_specularTint;
    float param_22 = qt_roughnessAmount;
    float param_23 = qt_metalnessAmount;
    vec3 param_24 = qt_f0;
    vec3 param_25 = qt_f90;
    qt_processPunctualLighting(param_14, param_15, param_16, param_17, param_18, param_19, param_20, param_21, param_22, param_23, param_24, param_25);
    global_diffuse_light.x = param_14.x;
    global_diffuse_light.y = param_14.y;
    global_diffuse_light.z = param_14.z;
    global_specular_light = param_15;
    global_diffuse_light = vec4(global_diffuse_light.xyz * qt_aoFactor, qt_objectOpacity * qt_diffuseColor.w);
    vec4 _2156 = global_diffuse_light;
    vec3 _2158 = _2156.xyz * (1.0 - qt_metalnessAmount);
    global_diffuse_light.x = _2158.x;
    global_diffuse_light.y = _2158.y;
    global_diffuse_light.z = _2158.z;
    vec4 qt_color_sum = vec4((global_diffuse_light.xyz + global_specular_light) + qt_global_emission, global_diffuse_light.w);
    vec4 param_26 = qt_color_sum;
    fragOutput = qt_tonemap(param_26);
}

