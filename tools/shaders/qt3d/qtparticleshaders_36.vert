#version 300 es
#ifdef GL_ARB_shader_draw_parameters
#extension GL_ARB_shader_draw_parameters : enable
#endif

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

struct cbMain
{
    float qt_alphaFade;
    float qt_billboard;
    vec2 qt_cameraProperties;
    uint qt_countPerSlice;
    vec3 qt_light_ambient_total;
    uint qt_lineSegmentCount;
    mat4 qt_modelMatrix;
    vec2 qt_oneOverParticleImageSize;
    float qt_opacity;
    mat4 qt_projectionMatrix;
    float qt_sizeModifier;
    vec4 qt_spriteConfig;
    float qt_texcoordScale;
    mat4 qt_viewMatrix;
};

uniform cbMain _38;

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
    uint v = index / _38.qt_countPerSlice;
    uint u = index - (_38.qt_countPerSlice * v);
    return (vec2(float(u * 4u), float(v)) + vec2(0.5)) * _38.qt_oneOverParticleImageSize;
}

Particle qt_loadParticle(uint index)
{
    uint param = index;
    vec2 offset = qt_indexToUV(param);
    vec4 p0 = textureLod(qt_particleTexture, offset, 0.0);
    vec4 p1 = textureLod(qt_particleTexture, vec2(offset.x + _38.qt_oneOverParticleImageSize.x, offset.y), 0.0);
    vec4 p2 = textureLod(qt_particleTexture, vec2(offset.x + (2.0 * _38.qt_oneOverParticleImageSize.x), offset.y), 0.0);
    vec4 p3 = textureLod(qt_particleTexture, vec2(offset.x + (3.0 * _38.qt_oneOverParticleImageSize.x), offset.y), 0.0);
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
    return pow(_38.qt_sizeModifier, float(segmentIndex));
}

float qt_calcAlphaFade(uint segmentIndex)
{
    return pow(_38.qt_alphaFade, float(segmentIndex));
}

float qt_ageToSpriteFactor(float age)
{
    return fract(age);
}

void main()
{
    uint segmentIndex = uint(gl_VertexID / 2);
    uint particleIndex = segmentIndex + (uint((gl_InstanceID + SPIRV_Cross_BaseInstance)) * _38.qt_lineSegmentCount);
    instanceIndex = particleIndex;
    uint param = particleIndex;
    Particle p = qt_loadParticle(param);
    float side = mod(float(gl_VertexID), 2.0) - 0.5;
    uint param_1 = segmentIndex;
    float size = p.size * qt_calcSizeFactor(param_1);
    vec3 o = (p.binormal * side) * size;
    vec4 worldPos = _38.qt_modelMatrix * vec4(p.position + (o * (1.0 - _38.qt_billboard)), 1.0);
    vec4 viewPos = _38.qt_viewMatrix * worldPos;
    vec3 viewBN = (transpose(inverse(_38.qt_viewMatrix * _38.qt_modelMatrix)) * vec4(p.binormal, 0.0)).xyz;
    viewBN.z = 0.0;
    viewBN = (normalize(viewBN) * side) * size;
    viewPos += (vec4(viewBN, 0.0) * _38.qt_billboard);
    texcoord.x = p.segmentLength * _38.qt_texcoordScale;
    texcoord.y = mod(float(gl_VertexID), 2.0);
    color = p.color;
    uint param_2 = segmentIndex;
    color.w *= (_38.qt_opacity * qt_calcAlphaFade(param_2));
    gl_Position = _38.qt_projectionMatrix * viewPos;
    float param_3 = p.age;
    spriteData.x = qt_ageToSpriteFactor(param_3);
    spriteData.y = p.animationFrame;
}

