#version 100
precision highp float;
precision highp int;

struct buf
{
    mat4 qt_Matrix;
    float qt_Opacity;
    vec4 color;
    vec3 iResolution;
    vec2 rectSize;
    float radius;
    float blur;
};

uniform buf _40;

varying vec2 fragCoord;
varying vec2 texCoord;

float roundedBox(vec2 centerPos, vec2 size, float radii)
{
    return length(max((abs(centerPos) - size) + vec2(radii), vec2(0.0))) - radii;
}

void main()
{
    vec2 param = fragCoord - (_40.iResolution.xy * 0.5);
    vec2 param_1 = _40.rectSize;
    float param_2 = _40.radius;
    float box = roundedBox(param, param_1, param_2);
    float a = 1.0 - smoothstep(0.0, _40.blur, box);
    gl_FragData[0] = ((_40.color * _40.qt_Opacity) * a) * a;
}

