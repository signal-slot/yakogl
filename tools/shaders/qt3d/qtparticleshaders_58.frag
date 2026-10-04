#version 300 es
precision highp float;
precision highp int;

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

uniform cbMain _47;

uniform highp sampler2D qt_sprite;
uniform highp sampler2D qt_particleTexture;

in vec2 spriteData;
in vec2 texcoord;
in vec4 color;
layout(location = 0) out vec4 fragOutput;
flat in uint instanceIndex;

vec4 qt_readColor()
{
    return color;
}

float qt_spriteCoords(inout vec2 coords[2])
{
    float imageOffset = (_47.qt_spriteConfig.x - _47.qt_spriteConfig.w) * spriteData.y;
    float factor = fract(imageOffset);
    vec2 imageOffsets = vec2(imageOffset - factor, (imageOffset - factor) + 1.0);
    imageOffsets = clamp(imageOffsets * _47.qt_spriteConfig.y, vec2(0.0), vec2(1.0));
    vec2 _97 = vec2(texcoord.x * _47.qt_spriteConfig.y, texcoord.y);
    coords[1] = _97;
    coords[0] = _97;
    coords[0].x += imageOffsets.x;
    coords[1].x += imageOffsets.y;
    return factor * _47.qt_spriteConfig.w;
}

vec3 qt_sRGBToLinear(vec3 srgbIn)
{
    return srgbIn * ((srgbIn * ((srgbIn * 0.305306017398834228515625) + vec3(0.6821711063385009765625))) + vec3(0.01252287812530994415283203125));
}

vec4 qt_sRGBToLinear(vec4 srgbIn)
{
    vec3 param = srgbIn.xyz;
    return vec4(qt_sRGBToLinear(param), srgbIn.w);
}

vec4 qt_readSprite()
{
    vec2 param[2];
    float _189 = qt_spriteCoords(param);
    vec2 coords[2] = param;
    float factor = _189;
    vec4 param_1 = texture(qt_sprite, coords[0]);
    vec4 param_2 = texture(qt_sprite, coords[1]);
    return mix(qt_sRGBToLinear(param_1), qt_sRGBToLinear(param_2), vec4(factor));
}

vec3 qt_linearTosRGB(vec3 color_1)
{
    vec3 S1 = sqrt(color_1);
    vec3 S2 = sqrt(S1);
    vec3 S3 = sqrt(S2);
    return ((S1 * 0.585122406482696533203125) + (S2 * 0.783140361309051513671875)) - (S3 * 0.3682627379894256591796875);
}

vec3 qt_tonemap(vec3 color_1)
{
    vec3 param = color_1;
    return qt_linearTosRGB(param);
}

vec4 qt_tonemap(vec4 color_1)
{
    vec3 param = color_1.xyz;
    return vec4(qt_tonemap(param), color_1.w);
}

void main()
{
    vec4 ret = qt_readColor() * qt_readSprite();
    vec4 param = ret;
    fragOutput = qt_tonemap(param);
}

