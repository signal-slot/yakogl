#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 matrix;
    mat4 gradientMatrix;
    vec2 translationPoint;
    float angle;
    float opacity;
};

uniform buf ubuf;

uniform highp sampler2D gradTabTexture;

varying vec2 coord;

void main()
{
    float t;
    if (abs(coord.y) == abs(coord.x))
    {
        t = (atan((-coord.y) + 0.00200000009499490261077880859375, coord.x) + ubuf.angle) * 0.15915493667125701904296875;
    }
    else
    {
        t = (atan(-coord.y, coord.x) + ubuf.angle) * 0.15915493667125701904296875;
    }
    gl_FragData[0] = texture2D(gradTabTexture, vec2(t - floor(t), 0.5)) * ubuf.opacity;
}

