#version 100
precision highp float;
precision highp int;

struct buf
{
    vec4 startColor;
    vec4 endColor;
};

uniform buf _22;

varying vec2 uv_coord;

void main()
{
    float gradientFactor = uv_coord.x;
    vec4 color = mix(_22.startColor, _22.endColor, vec4(gradientFactor));
    gl_FragData[0] = color;
}

