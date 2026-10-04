#version 300 es
#ifdef GL_ARB_shader_draw_parameters
#extension GL_ARB_shader_draw_parameters : enable
#endif

struct Particle
{
    vec3 position;
    float size;
    vec3 rotation;
    float age;
    vec4 color;
};

const vec2 _235[4] = vec2[](vec2(0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(1.0));

struct cbMain
{
    float qt_billboard;
    uint qt_countPerSlice;
    vec3 qt_light_ambient_total;
    mat4 qt_modelMatrix;
    vec2 qt_oneOverParticleImageSize;
    float qt_opacity;
    mat4 qt_projectionMatrix;
    vec4 qt_spriteConfig;
    mat4 qt_viewMatrix;
};

uniform cbMain _32;

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
    uint v = index / _32.qt_countPerSlice;
    uint u = index - (_32.qt_countPerSlice * v);
    return (vec2(float(u * 3u), float(v)) + vec2(0.5)) * _32.qt_oneOverParticleImageSize;
}

Particle qt_loadParticle(uint index)
{
    uint param = index;
    vec2 offset = qt_indexToUV(param);
    vec4 p0 = textureLod(qt_particleTexture, offset, 0.0);
    vec4 p1 = textureLod(qt_particleTexture, vec2(offset.x + _32.qt_oneOverParticleImageSize.x, offset.y), 0.0);
    vec4 p2 = textureLod(qt_particleTexture, vec2(offset.x + (2.0 * _32.qt_oneOverParticleImageSize.x), offset.y), 0.0);
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

void main()
{
    uint particleIndex = uint((gl_InstanceID + SPIRV_Cross_BaseInstance));
    uint cornerIndex = uint(gl_VertexID);
    vec2 corner = _235[cornerIndex];
    instanceIndex = particleIndex;
    uint param = particleIndex;
    Particle p = qt_loadParticle(param);
    vec3 param_1 = p.rotation;
    mat3 rotMat = qt_fromEulerRotation(param_1);
    vec4 worldPos = _32.qt_modelMatrix * vec4(p.position, 1.0);
    vec4 viewPos = worldPos;
    vec3 offset = vec3(corner - vec2(0.5), 0.0) * p.size;
    vec4 _282 = viewPos;
    vec3 _284 = _282.xyz + ((offset * rotMat) * (1.0 - _32.qt_billboard));
    viewPos.x = _284.x;
    viewPos.y = _284.y;
    viewPos.z = _284.z;
    viewPos = _32.qt_viewMatrix * viewPos;
    vec4 _302 = viewPos;
    vec3 _304 = _302.xyz + ((rotMat * offset) * _32.qt_billboard);
    viewPos.x = _304.x;
    viewPos.y = _304.y;
    viewPos.z = _304.z;
    texcoord = corner;
    color = p.color;
    color.w *= _32.qt_opacity;
    gl_Position = _32.qt_projectionMatrix * viewPos;
}

