#version 300 es

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

uniform cbMain _69;

uniform highp sampler2D qt_inputTexture;
uniform highp sampler2D tex;

out vec2 qt_inputUV;
layout(location = 1) in vec2 attr_uv;
out vec2 qt_textureUV;
layout(location = 0) in vec3 attr_pos;
flat out uint qt_viewIndex;

vec2 qt_effectTextureMapUV(vec2 uv)
{
    return uv;
}

void qt_customMain(vec3 VERTEX)
{
}

void main()
{
    qt_inputUV = attr_uv;
    vec2 param = attr_uv;
    qt_textureUV = qt_effectTextureMapUV(param);
    vec4 qt_vertPosition = vec4(attr_pos, 1.0);
    qt_viewIndex = 0u;
    vec3 param_1 = qt_vertPosition.xyz;
    qt_customMain(param_1);
    qt_vertPosition.x = param_1.x;
    qt_vertPosition.y = param_1.y;
    qt_vertPosition.z = param_1.z;
    gl_Position = _69.qt_modelViewProjection * qt_vertPosition;
}

