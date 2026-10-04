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
    vec3 rotation;
    float age;
    vec4 color;
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

const vec2 _514[4] = vec2[](vec2(0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(1.0));

struct cbMain
{
    ParticleLightData lightData;
    float qt_billboard;
    vec2 qt_cameraProperties;
    uint qt_countPerSlice;
    vec3 qt_light_ambient_total;
    mat4 qt_modelMatrix;
    vec2 qt_oneOverParticleImageSize;
    float qt_opacity;
    uint qt_pointLights;
    mat4 qt_projectionMatrix[2];
    uint qt_spotLights;
    vec4 qt_spriteConfig;
    mat4 qt_viewMatrix[2];
};

uniform cbMain _83;

uniform highp sampler2D qt_particleTexture;
uniform highp sampler2D qt_colorTable;
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
    uint v = index / _83.qt_countPerSlice;
    uint u = index - (_83.qt_countPerSlice * v);
    return (vec2(float(u * 3u), float(v)) + vec2(0.5)) * _83.qt_oneOverParticleImageSize;
}

Particle qt_loadParticle(uint index)
{
    uint param = index;
    vec2 offset = qt_indexToUV(param);
    vec4 p0 = textureLod(qt_particleTexture, offset, 0.0);
    vec4 p1 = textureLod(qt_particleTexture, vec2(offset.x + _83.qt_oneOverParticleImageSize.x, offset.y), 0.0);
    vec4 p2 = textureLod(qt_particleTexture, vec2(offset.x + (2.0 * _83.qt_oneOverParticleImageSize.x), offset.y), 0.0);
    Particle p;
    p.position = p0.xyz;
    p.size = p0.w;
    p.rotation = p1.xyz;
    p.age = p1.w;
    p.color = p2;
    return p;
}

mat3 qt_fromEulerRotation(vec3 rotation)
{
    float a = cos(rotation.x);
    float b = sin(rotation.x);
    float c = cos(rotation.y);
    float d = sin(rotation.y);
    float e = cos(rotation.z);
    float f = sin(rotation.z);
    float bd = b * d;
    float ad = a * d;
    mat3 ret;
    ret[0].x = c * e;
    ret[0].y = (-c) * f;
    ret[0].z = d;
    ret[1].x = (bd * e) + (a * f);
    ret[1].y = (a * e) - (bd * f);
    ret[1].z = (-b) * c;
    ret[2].x = (b * f) - (ad * e);
    ret[2].y = (ad * f) + (b * e);
    ret[2].z = a * c;
    return ret;
}

vec4 qt_attenuate4(vec4 dist, vec4 catt, vec4 latt, vec4 qatt)
{
    vec4 factors = (catt + (dist * latt)) + ((dist * dist) * qatt);
    return vec4(1.0) / factors;
}

vec3 qt_calcPointLights(vec3 position)
{
    vec4 lengths = vec4(length(position - _83.lightData.qt_pointLightPosition[0].xyz), length(position - _83.lightData.qt_pointLightPosition[1].xyz), length(position - _83.lightData.qt_pointLightPosition[2].xyz), length(position - _83.lightData.qt_pointLightPosition[3].xyz));
    vec4 param = lengths;
    vec4 param_1 = _83.lightData.qt_pointLightConstantAtt;
    vec4 param_2 = _83.lightData.qt_pointLightLinearAtt;
    vec4 param_3 = _83.lightData.qt_pointLightQuadAtt;
    vec4 attenuations = qt_attenuate4(param, param_1, param_2, param_3);
    return (((_83.lightData.qt_pointLightColor[0].xyz * attenuations.x) + (_83.lightData.qt_pointLightColor[1].xyz * attenuations.y)) + (_83.lightData.qt_pointLightColor[2].xyz * attenuations.z)) + (_83.lightData.qt_pointLightColor[3].xyz * attenuations.w);
}

vec3 qt_calcSpotLights(vec3 position)
{
    vec3 lightVector0 = position - _83.lightData.qt_spotLightPosition[0].xyz;
    vec3 lightVector1 = position - _83.lightData.qt_spotLightPosition[1].xyz;
    vec3 lightVector2 = position - _83.lightData.qt_spotLightPosition[2].xyz;
    vec3 lightVector3 = position - _83.lightData.qt_spotLightPosition[3].xyz;
    vec4 lengths = vec4(length(lightVector0), length(lightVector1), length(lightVector2), length(lightVector3));
    vec4 param = lengths;
    vec4 param_1 = _83.lightData.qt_spotLightConstantAtt;
    vec4 param_2 = _83.lightData.qt_spotLightLinearAtt;
    vec4 param_3 = _83.lightData.qt_spotLightQuadAtt;
    vec4 attenuations = qt_attenuate4(param, param_1, param_2, param_3);
    vec4 spots = vec4(acos(dot(normalize(lightVector0), normalize(_83.lightData.qt_spotLightDirection[0].xyz))), acos(dot(normalize(lightVector1), normalize(_83.lightData.qt_spotLightDirection[1].xyz))), acos(dot(normalize(lightVector2), normalize(_83.lightData.qt_spotLightDirection[2].xyz))), acos(dot(normalize(lightVector3), normalize(_83.lightData.qt_spotLightDirection[3].xyz))));
    vec4 cones = vec4(1.0) - smoothstep(_83.lightData.qt_spotLightInnerConeAngle, _83.lightData.qt_spotLightConeAngle, spots);
    attenuations *= cones;
    return (((_83.lightData.qt_spotLightColor[0].xyz * attenuations.x) + (_83.lightData.qt_spotLightColor[1].xyz * attenuations.y)) + (_83.lightData.qt_spotLightColor[2].xyz * attenuations.z)) + (_83.lightData.qt_spotLightColor[3].xyz * attenuations.w);
}

vec3 qt_calcLightColor(vec3 position)
{
    vec3 color_1 = vec3(0.0);
    if (_83.qt_pointLights != 0u)
    {
        vec3 param = position;
        color_1 += qt_calcPointLights(param);
    }
    if (_83.qt_spotLights != 0u)
    {
        vec3 param_1 = position;
        color_1 += qt_calcSpotLights(param_1);
    }
    return color_1 + _83.qt_light_ambient_total;
}

float qt_ageToSpriteFactor(float age)
{
    return fract(age);
}

void main()
{
    uint particleIndex = uint((gl_InstanceID + SPIRV_Cross_BaseInstance));
    uint cornerIndex = uint(gl_VertexID);
    vec2 corner = _514[cornerIndex];
    instanceIndex = particleIndex;
    uint param = particleIndex;
    Particle p = qt_loadParticle(param);
    vec3 param_1 = p.rotation;
    mat3 rotMat = qt_fromEulerRotation(param_1);
    vec4 worldPos = _83.qt_modelMatrix * vec4(p.position, 1.0);
    vec4 viewPos = worldPos;
    vec3 offset = vec3(corner - vec2(0.5), 0.0) * p.size;
    vec4 _561 = viewPos;
    vec3 _563 = _561.xyz + ((offset * rotMat) * (1.0 - _83.qt_billboard));
    viewPos.x = _563.x;
    viewPos.y = _563.y;
    viewPos.z = _563.z;
    viewPos = _83.qt_viewMatrix[gl_ViewID_OVR] * viewPos;
    vec4 _582 = viewPos;
    vec3 _584 = _582.xyz + ((rotMat * offset) * _83.qt_billboard);
    viewPos.x = _584.x;
    viewPos.y = _584.y;
    viewPos.z = _584.z;
    texcoord = corner;
    color = p.color;
    color.w *= _83.qt_opacity;
    vec3 param_2 = worldPos.xyz;
    vec4 _609 = color;
    vec3 _611 = _609.xyz * qt_calcLightColor(param_2);
    color.x = _611.x;
    color.y = _611.y;
    color.z = _611.z;
    gl_Position = _83.qt_projectionMatrix[gl_ViewID_OVR] * viewPos;
    float param_3 = p.age;
    spriteData.x = qt_ageToSpriteFactor(param_3);
}

