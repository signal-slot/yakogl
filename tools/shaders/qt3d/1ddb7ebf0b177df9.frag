#version 300 es
precision highp float;
precision highp int;

struct cbMain
{
    vec2 qt_cameraProperties;
    float qt_fps;
    float qt_frame_num;
    vec2 qt_inputSize;
    mat4 qt_modelViewProjection;
    float qt_nearClipValue;
    float qt_normalAdjustViewportFactor;
    vec2 qt_outputSize;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
};

uniform cbMain _97;

uniform highp sampler2D tex;
uniform highp sampler2D qt_inputTexture;

in vec2 qt_textureUV;
layout(location = 0) out vec4 fragOutput;
in vec2 qt_inputUV;
flat in uint qt_viewIndex;

void qt_customMain()
{
    vec4 c = texture(tex, qt_textureUV);
    fragOutput = c * texture(qt_inputTexture, qt_inputUV);
}

vec3 qt_linearTosRGB(vec3 color)
{
    vec3 S1 = sqrt(color);
    vec3 S2 = sqrt(S1);
    vec3 S3 = sqrt(S2);
    return ((S1 * 0.5851224064826965) + (S2 * 0.7831403613090515)) - (S3 * 0.36826273798942566);
}

vec3 qt_tonemap(vec3 color)
{
    vec3 param = color;
    return qt_linearTosRGB(param);
}

vec4 qt_tonemap(vec4 color)
{
    vec3 param = color.xyz;
    return vec4(qt_tonemap(param), color.w);
}

void main()
{
    qt_customMain();
    vec4 param = fragOutput;
    fragOutput = qt_tonemap(param);
}

