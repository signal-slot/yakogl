#version 300 es
precision highp float;
precision highp int;

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
    vec2 qt_cameraProperties;
    uint qt_countPerSlice;
    vec3 qt_light_ambient_total;
    uint qt_lineSegmentCount;
    mat4 qt_modelMatrix;
    vec2 qt_oneOverParticleImageSize;
    float qt_opacity;
    uint qt_pointLights;
    mat4 qt_projectionMatrix;
    float qt_sizeModifier;
    uint qt_spotLights;
    vec4 qt_spriteConfig;
    float qt_texcoordScale;
    mat4 qt_viewMatrix;
};

uniform cbMain _75;

uniform highp sampler2D qt_sprite;
uniform highp sampler2D qt_particleTexture;

in vec2 spriteData;
in vec2 texcoord;
in vec4 color;
layout(location = 0) out vec4 fragOutput;
layout(location = 1) out vec4 revealageOutput;
flat in uint instanceIndex;

vec4 qt_readColor()
{
    return color;
}

float qt_spriteCoords(inout vec2 coords[2])
{
    float imageOffset = (_75.qt_spriteConfig.x - _75.qt_spriteConfig.w) * spriteData.y;
    float factor = fract(imageOffset);
    vec2 imageOffsets = vec2(imageOffset - factor, (imageOffset - factor) + 1.0);
    imageOffsets = clamp(imageOffsets * _75.qt_spriteConfig.y, vec2(0.0), vec2(1.0));
    vec2 _124 = vec2(texcoord.x * _75.qt_spriteConfig.y, texcoord.y);
    coords[1] = _124;
    coords[0] = _124;
    coords[0].x += imageOffsets.x;
    coords[1].x += imageOffsets.y;
    return factor * _75.qt_spriteConfig.w;
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
    float _216 = qt_spriteCoords(param);
    vec2 coords[2] = param;
    float factor = _216;
    vec4 param_1 = texture(qt_sprite, coords[0]);
    vec4 param_2 = texture(qt_sprite, coords[1]);
    return mix(qt_sRGBToLinear(param_1), qt_sRGBToLinear(param_2), vec4(factor));
}

float qt_transparencyWeight(float z, float a, float depthRange)
{
    return pow(a, 1.0) * clamp(0.300000011920928955078125 / (9.9999997473787516355514526367188e-06 + pow(z / depthRange, 4.0)), 0.00999999977648258209228515625, 3000.0);
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
    float z = abs(gl_FragCoord.z);
    float distWeight = qt_transparencyWeight(z, ret.w, _75.qt_cameraProperties.y);
    vec4 param = ret;
    fragOutput = qt_tonemap(param) * distWeight;
    revealageOutput = vec4(ret.w);
}

