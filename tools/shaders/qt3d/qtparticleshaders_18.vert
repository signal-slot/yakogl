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

const vec2 _246[4] = vec2[](vec2(0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(1.0));

struct cbMain
{
    float qt_billboard;
    vec2 qt_cameraProperties;
    uint qt_countPerSlice;
    vec3 qt_light_ambient_total;
    mat4 qt_modelMatrix;
    vec2 qt_oneOverParticleImageSize;
    float qt_opacity;
    mat4 qt_projectionMatrix[2];
    vec4 qt_spriteConfig;
    mat4 qt_viewMatrix[2];
};

uniform cbMain _40;

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
    uint v = index / _40.qt_countPerSlice;
    uint u = index - (_40.qt_countPerSlice * v);
    return (vec2(float(u * 3u), float(v)) + vec2(0.5)) * _40.qt_oneOverParticleImageSize;
}

Particle qt_loadParticle(uint index)
{
    uint param = index;
    vec2 offset = qt_indexToUV(param);
    vec4 p0 = textureLod(qt_particleTexture, offset, 0.0);
    vec4 p1 = textureLod(qt_particleTexture, vec2(offset.x + _40.qt_oneOverParticleImageSize.x, offset.y), 0.0);
    vec4 p2 = textureLod(qt_particleTexture, vec2(offset.x + (2.0 * _40.qt_oneOverParticleImageSize.x), offset.y), 0.0);
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

float qt_ageToSpriteFactor(float age)
{
    return fract(age);
}

void main()
{
    uint particleIndex = uint((gl_InstanceID + SPIRV_Cross_BaseInstance));
    uint cornerIndex = uint(gl_VertexID);
    vec2 corner = _246[cornerIndex];
    instanceIndex = particleIndex;
    uint param = particleIndex;
    Particle p = qt_loadParticle(param);
    vec3 param_1 = p.rotation;
    mat3 rotMat = qt_fromEulerRotation(param_1);
    vec4 worldPos = _40.qt_modelMatrix * vec4(p.position, 1.0);
    vec4 viewPos = worldPos;
    vec3 offset = vec3(corner - vec2(0.5), 0.0) * p.size;
    vec4 _293 = viewPos;
    vec3 _295 = _293.xyz + ((offset * rotMat) * (1.0 - _40.qt_billboard));
    viewPos.x = _295.x;
    viewPos.y = _295.y;
    viewPos.z = _295.z;
    viewPos = _40.qt_viewMatrix[gl_ViewID_OVR] * viewPos;
    vec4 _315 = viewPos;
    vec3 _317 = _315.xyz + ((rotMat * offset) * _40.qt_billboard);
    viewPos.x = _317.x;
    viewPos.y = _317.y;
    viewPos.z = _317.z;
    texcoord = corner;
    color = p.color;
    color.w *= _40.qt_opacity;
    gl_Position = _40.qt_projectionMatrix[gl_ViewID_OVR] * viewPos;
    float param_2 = p.age;
    spriteData.x = qt_ageToSpriteFactor(param_2);
}

