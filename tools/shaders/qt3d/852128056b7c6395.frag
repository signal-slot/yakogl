#version 300 es
precision highp float;
precision highp int;

struct cbMain
{
    float alpha;
    float amplitude;
    vec3 qt_cameraDirection;
    vec3 qt_cameraPosition;
    vec2 qt_cameraProperties;
    vec4 qt_material_properties;
    mat4 qt_modelMatrix;
    mat4 qt_modelViewProjection;
    mat3 qt_normalMatrix;
    vec4 qt_rhi_properties;
    mat4 qt_viewMatrix;
    mat4 qt_viewProjectionMatrix;
    uint textureFromItem;
    uint texturing;
    float time;
};

uniform cbMain _39;

uniform highp sampler2D tex;

in vec3 pos;
layout(location = 0) out vec4 fragOutput;
in vec2 coord;

void qt_customMain()
{
    vec3 rgb = vec3(pos.x * 0.019999999552965164, pos.y * 0.019999999552965164, pos.z * 0.019999999552965164);
    fragOutput = vec4(rgb, _39.alpha);
}

void main()
{
    qt_customMain();
}

