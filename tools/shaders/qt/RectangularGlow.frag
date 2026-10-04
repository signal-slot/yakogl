#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float qt_Opacity;
    float relativeSizeX;
    float relativeSizeY;
    float spread;
    vec4 color;
};

uniform buf ubuf;

varying vec2 qt_TexCoord0;

float linearstep(float e0, float e1, float x)
{
    return clamp((x - e0) / (e1 - e0), 0.0, 1.0);
}

void main()
{
    float alpha = smoothstep(0.0, ubuf.relativeSizeX, 0.5 - abs(0.5 - qt_TexCoord0.x)) * smoothstep(0.0, ubuf.relativeSizeY, 0.5 - abs(0.5 - qt_TexCoord0.y));
    float param = ubuf.spread;
    float param_1 = 1.0 - ubuf.spread;
    float param_2 = alpha;
    float spreadMultiplier = linearstep(param, param_1, param_2);
    gl_FragData[0] = ((ubuf.color * ubuf.qt_Opacity) * spreadMultiplier) * spreadMultiplier;
}

