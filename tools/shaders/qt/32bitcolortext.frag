#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 modelViewMatrix;
    mat4 projectionMatrix;
    vec2 textureScale;
    float dpr;
    vec4 color;
};

uniform buf _23;

uniform highp sampler2D _qt_texture;

varying vec2 sampleCoord;

void main()
{
    gl_FragData[0] = texture2D(_qt_texture, sampleCoord) * _23.color.w;
}

