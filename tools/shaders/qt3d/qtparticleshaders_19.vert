#version 300 es
#extension GL_OVR_multiview2 : require
#ifdef GL_ARB_shader_draw_parameters
#extension GL_ARB_shader_draw_parameters : enable
#endif
layout(num_views = 2) in;

struct Particle
{
    vec3 position;
    float size;
    vec4 color;
    vec3 binormal;
    float animationFrame;
    float age;
    float segmentLength;
    vec2 unusedPadding;
};

struct ParticleLightData
{
    vec4 qt_pointLightPosition[4];
    vec4 qt_pointLightConstantAtt;
    vec4 qt_pointLightLinearAtt;
    vec4 qt_pointLightQuadAtt;
    vec4 qt_pointLightColor[4];
    vec4 qt_spotLightPosition[4];
    vec4 qt_spotLightConstantAtt;
    vec4 qt_spotLightLinearAtt;
    vec4 qt_spotLightQuadAtt;
    vec4 qt_spotLightColor[4];
    vec4 qt_spotLightDirection[4];
    vec4 qt_spotLightConeAngle;
    vec4 qt_spotLightInnerConeAngle;
};

struct cbMain
{
    ParticleLightData lightData;
    float qt_alphaFade;
    float qt_billboard;
    uint qt_countPerSlice;
    vec3 qt_light_ambient_total;
    uint qt_lineSegmentCount;
    mat4 qt_modelMatrix;
    vec2 qt_oneOverParticleImageSize;
    float qt_opacity;
    uint qt_pointLights;
    mat4 qt_projectionMatrix[2];
    float qt_sizeModifier;
    uint qt_spotLights;
    vec4 qt_spriteConfig;
    float qt_texcoordScale;
    mat4 qt_viewMatrix[2];
};

uniform cbMain _80;

uniform highp sampler2D qt_particleTexture;
uniform highp sampler2D qt_sprite;

#ifdef GL_ARB_shader_draw_parameters
#define SPIRV_Cross_BaseInstance gl_BaseInstanceARB
#else
uniform int SPIRV_Cross_BaseInstance;
#endif
flat out uint instanceIndex;
out vec2 texcoord;
out vec4 color;
out vec2 spriteData;

vec2 qt_indexToUV(uint index)
{
    uint v = index / _80.qt_countPerSlice;
    uint u = index - (_80.qt_countPerSlice * v);
    return (vec2(float(u * 4u), float(v)) + vec2(0.5)) * _80.qt_oneOverParticleImageSize;
}

Particle qt_loadParticle(uint index)
{
    uint param = index;
    vec2 offset = qt_indexToUV(param);
    vec4 p0 = textureLod(qt_particleTexture, offset, 0.0);
    vec4 p1 = textureLod(qt_particleTexture, vec2(offset.x + _80.qt_oneOverParticleImageSize.x, offset.y), 0.0);
    vec4 p2 = textureLod(qt_particleTexture, vec2(offset.x + (2.0 * _80.qt_oneOverParticleImageSize.x), offset.y), 0.0);
    vec4 p3 = textureLod(qt_particleTexture, vec2(offset.x + (3.0 * _80.qt_oneOverParticleImageSize.x), offset.y), 0.0);
    Particle p;
    p.position = p0.xyz;
    p.size = p0.w;
    p.color = p1;
    p.binormal = p2.xyz;
    p.animationFrame = p2.w;
    p.age = p3.x;
    p.segmentLength = p3.y;
    return p;
}

float qt_calcSizeFactor(uint segmentIndex)
{
    return pow(_80.qt_sizeModifier, float(segmentIndex));
}

float qt_calcAlphaFade(uint segmentIndex)
{
    return pow(_80.qt_alphaFade, float(segmentIndex));
}

vec4 qt_attenuate4(vec4 dist, vec4 catt, vec4 latt, vec4 qatt)
{
    vec4 factors = (catt + (dist * latt)) + ((dist * dist) * qatt);
    return vec4(1.0) / factors;
}

vec3 qt_calcPointLights(vec3 position)
{
    vec4 lengths = vec4(length(position - _80.lightData.qt_pointLightPosition[0].xyz), length(position - _80.lightData.qt_pointLightPosition[1].xyz), length(position - _80.lightData.qt_pointLightPosition[2].xyz), length(position - _80.lightData.qt_pointLightPosition[3].xyz));
    vec4 param = lengths;
    vec4 param_1 = _80.lightData.qt_pointLightConstantAtt;
    vec4 param_2 = _80.lightData.qt_pointLightLinearAtt;
    vec4 param_3 = _80.lightData.qt_pointLightQuadAtt;
    vec4 attenuations = qt_attenuate4(param, param_1, param_2, param_3);
    return (((_80.lightData.qt_pointLightColor[0].xyz * attenuations.x) + (_80.lightData.qt_pointLightColor[1].xyz * attenuations.y)) + (_80.lightData.qt_pointLightColor[2].xyz * attenuations.z)) + (_80.lightData.qt_pointLightColor[3].xyz * attenuations.w);
}

vec3 qt_calcSpotLights(vec3 position)
{
    vec3 lightVector0 = position - _80.lightData.qt_spotLightPosition[0].xyz;
    vec3 lightVector1 = position - _80.lightData.qt_spotLightPosition[1].xyz;
    vec3 lightVector2 = position - _80.lightData.qt_spotLightPosition[2].xyz;
    vec3 lightVector3 = position - _80.lightData.qt_spotLightPosition[3].xyz;
    vec4 lengths = vec4(length(lightVector0), length(lightVector1), length(lightVector2), length(lightVector3));
    vec4 param = lengths;
    vec4 param_1 = _80.lightData.qt_spotLightConstantAtt;
    vec4 param_2 = _80.lightData.qt_spotLightLinearAtt;
    vec4 param_3 = _80.lightData.qt_spotLightQuadAtt;
    vec4 attenuations = qt_attenuate4(param, param_1, param_2, param_3);
    vec4 spots = vec4(acos(dot(normalize(lightVector0), normalize(_80.lightData.qt_spotLightDirection[0].xyz))), acos(dot(normalize(lightVector1), normalize(_80.lightData.qt_spotLightDirection[1].xyz))), acos(dot(normalize(lightVector2), normalize(_80.lightData.qt_spotLightDirection[2].xyz))), acos(dot(normalize(lightVector3), normalize(_80.lightData.qt_spotLightDirection[3].xyz))));
    vec4 cones = vec4(1.0) - smoothstep(_80.lightData.qt_spotLightInnerConeAngle, _80.lightData.qt_spotLightConeAngle, spots);
    attenuations *= cones;
    return (((_80.lightData.qt_spotLightColor[0].xyz * attenuations.x) + (_80.lightData.qt_spotLightColor[1].xyz * attenuations.y)) + (_80.lightData.qt_spotLightColor[2].xyz * attenuations.z)) + (_80.lightData.qt_spotLightColor[3].xyz * attenuations.w);
}

vec3 qt_calcLightColor(vec3 position)
{
    vec3 color_1 = vec3(0.0);
    if (_80.qt_pointLights != 0u)
    {
        vec3 param = position;
        color_1 += qt_calcPointLights(param);
    }
    if (_80.qt_spotLights != 0u)
    {
        vec3 param_1 = position;
        color_1 += qt_calcSpotLights(param_1);
    }
    return color_1 + _80.qt_light_ambient_total;
}

void main()
{
    uint segmentIndex = uint(gl_VertexID / 2);
    uint particleIndex = segmentIndex + (uint((gl_InstanceID + SPIRV_Cross_BaseInstance)) * _80.qt_lineSegmentCount);
    instanceIndex = particleIndex;
    uint param = particleIndex;
    Particle p = qt_loadParticle(param);
    float side = mod(float(gl_VertexID), 2.0) - 0.5;
    uint param_1 = segmentIndex;
    float size = p.size * qt_calcSizeFactor(param_1);
    vec3 o = (p.binormal * side) * size;
    vec4 worldPos = _80.qt_modelMatrix * vec4(p.position + (o * (1.0 - _80.qt_billboard)), 1.0);
    vec4 viewPos = _80.qt_viewMatrix[gl_ViewID_OVR] * worldPos;
    vec3 viewBN = (transpose(inverse(_80.qt_viewMatrix[gl_ViewID_OVR] * _80.qt_modelMatrix)) * vec4(p.binormal, 0.0)).xyz;
    viewBN.z = 0.0;
    viewBN = (normalize(viewBN) * side) * size;
    viewPos += (vec4(viewBN, 0.0) * _80.qt_billboard);
    texcoord.x = p.segmentLength * _80.qt_texcoordScale;
    texcoord.y = mod(float(gl_VertexID), 2.0);
    color = p.color;
    uint param_2 = segmentIndex;
    color.w *= (_80.qt_opacity * qt_calcAlphaFade(param_2));
    vec3 param_3 = worldPos.xyz;
    vec4 _569 = color;
    vec3 _571 = _569.xyz * qt_calcLightColor(param_3);
    color.x = _571.x;
    color.y = _571.y;
    color.z = _571.z;
    gl_Position = _80.qt_projectionMatrix[gl_ViewID_OVR] * viewPos;
    spriteData.y = p.animationFrame;
}

