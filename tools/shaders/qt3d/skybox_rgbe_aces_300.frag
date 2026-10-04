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

vec3 qt_linearTosRGB(vec3 color)
{
    vec3 S1 = sqrt(color);
    vec3 S2 = sqrt(S1);
    vec3 S3 = sqrt(S2);
    return ((S1 * 0.585122406482696533203125) + (S2 * 0.783140361309051513671875)) - (S3 * 0.3682627379894256591796875);
}

vec3 qt_toneMapACES(vec3 color)
{
    vec3 param = clamp((color * ((color * 2.5099999904632568359375) + vec3(0.02999999932944774627685546875))) / ((color * ((color * 2.4300000667572021484375) + vec3(0.589999973773956298828125))) + vec3(0.14000000059604644775390625)), vec3(0.0), vec3(1.0));
    return qt_linearTosRGB(param);
}

vec3 qt_tonemap(vec3 color)
{
    vec3 param = color;
    return qt_toneMapACES(param);
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

