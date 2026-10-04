#version 300 es
precision highp float;
precision highp int;

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

uniform cbMain _199;

uniform highp sampler2D qt_sprite;
uniform highp sampler2D qt_colorTable;
uniform highp sampler2D qt_particleTexture;

in vec2 texcoord;
in vec4 color;
in vec2 spriteData;
flat in uint instanceIndex;
layout(location = 0) out vec4 fragOutput;
layout(location = 1) out vec4 revealageOutput;

float qt_rand(vec2 co)
{
    return fract(sin(dot(co, vec2(12.98980045318603515625, 78.233001708984375))) * 43758.546875);
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

vec4 qt_readColor()
{
    vec2 param = vec2(float(instanceIndex), 0.0);
    vec4 param_1 = texture(qt_colorTable, vec2(spriteData.x, qt_rand(param)));
    return color * qt_sRGBToLinear(param_1);
}

vec4 qt_readSprite()
{
    vec4 param = texture(qt_sprite, texcoord);
    return qt_sRGBToLinear(param);
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
    float distWeight = qt_transparencyWeight(z, ret.w, _199.qt_cameraProperties.y);
    vec4 param = ret;
    fragOutput = qt_tonemap(param) * distWeight;
    revealageOutput = vec4(ret.w);
}

