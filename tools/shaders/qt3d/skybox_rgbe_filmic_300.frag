#version 300 es
precision highp float;
precision highp int;

struct buf
{
    vec4 skyboxProperties;
    mat3 orientation;
    mat4 viewProjection;
    mat4 inverseProjection;
    mat3 viewMatrix;
};

uniform buf ubuf;

uniform highp samplerCube skybox_image;

in vec3 eye_direction;
layout(location = 0) out vec4 fragOutput;

vec3 qt_exposure(vec3 color, float exposure)
{
    return vec3(1.0) - exp((-color) * exposure);
}

vec3 qt_toneMapFilmicSub(vec3 color)
{
    return (((color * ((color * 0.1500000059604644775390625) + vec3(0.0500000007450580596923828125))) + vec3(0.0040000001899898052215576171875)) / ((color * ((color * 0.1500000059604644775390625) + vec3(0.5))) + vec3(0.0599999986588954925537109375))) - vec3(0.066666670143604278564453125);
}

vec3 qt_linearTosRGB(vec3 color)
{
    vec3 S1 = sqrt(color);
    vec3 S2 = sqrt(S1);
    vec3 S3 = sqrt(S2);
    return ((S1 * 0.585122406482696533203125) + (S2 * 0.783140361309051513671875)) - (S3 * 0.3682627379894256591796875);
}

vec3 qt_toneMapFilmic(inout vec3 color)
{
    vec3 param = color * 2.0;
    color = qt_toneMapFilmicSub(param);
    vec3 param_1 = vec3(11.19999980926513671875);
    vec3 whiteScale = vec3(1.0) / qt_toneMapFilmicSub(param_1);
    vec3 param_2 = color * whiteScale;
    return qt_linearTosRGB(param_2);
}

vec3 qt_tonemap(vec3 color)
{
    vec3 param = color;
    vec3 _99 = qt_toneMapFilmic(param);
    return _99;
}

void main()
{
    vec3 eye = normalize(eye_direction);
    float mipLevel = mix(0.0, ubuf.skyboxProperties.w, ubuf.skyboxProperties.z);
    vec4 color = textureLod(skybox_image, eye, mipLevel);
    color = vec4(color.xyz * pow(2.0, (color.w * 255.0) - 128.0), 1.0);
    vec3 param = color.xyz;
    float param_1 = ubuf.skyboxProperties.y;
    vec3 param_2 = qt_exposure(param, param_1);
    fragOutput = vec4(qt_tonemap(param_2), 1.0);
}

