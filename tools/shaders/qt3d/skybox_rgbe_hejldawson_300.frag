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

vec3 qt_tonemapHejlDawson(inout vec3 color)
{
    color = max(vec3(0.0), color - vec3(0.0040000001899898052215576171875));
    return (color * ((color * 6.19999980926513671875) + vec3(0.5))) / ((color * ((color * 6.19999980926513671875) + vec3(1.7000000476837158203125))) + vec3(0.0599999986588954925537109375));
}

vec3 qt_tonemap(vec3 color)
{
    vec3 param = color;
    vec3 _52 = qt_tonemapHejlDawson(param);
    return _52;
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

