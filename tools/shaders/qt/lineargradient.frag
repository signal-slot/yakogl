#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 matrix;
    mat4 gradientMatrix;
    vec2 gradStart;
    vec2 gradEnd;
    float opacity;
};

uniform buf ubuf;

uniform highp sampler2D gradTabTexture;

varying float gradTabIndex;

void main()
{
    gl_FragData[0] = texture2D(gradTabTexture, vec2(gradTabIndex, 0.5)) * ubuf.opacity;
}

