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

vec3 qt_tonemap(vec3 color)
{
    return color;
}

void main()
{
    vec3 eye = normalize(eye_direction);
    float mipLevel = mix(0.0, ubuf.skyboxProperties.w, ubuf.skyboxProperties.z);
    vec4 color = textureLod(skybox_image, eye, mipLevel);
    vec3 param = color.xyz;
    float param_1 = ubuf.skyboxProperties.y;
    vec3 param_2 = qt_exposure(param, param_1);
    fragOutput = vec4(qt_tonemap(param_2), 1.0);
}

