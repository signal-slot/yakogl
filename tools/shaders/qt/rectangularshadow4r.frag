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
    vec4 radius4;
    float blur;
};

uniform buf _63;

varying vec2 fragCoord;
varying vec2 texCoord;

float roundedBox4R(vec2 centerPos, vec2 size, vec4 radii)
{
    int i = int((2.0 + (0.5 * sign(centerPos.x))) + sign(centerPos.y));
    return length(max((abs(centerPos) - size) + vec2(radii[i]), vec2(0.0))) - radii[i];
}

void main()
{
    vec2 param = fragCoord - (_63.iResolution.xy * 0.5);
    vec2 param_1 = _63.rectSize;
    vec4 param_2 = _63.radius4;
    float box = roundedBox4R(param, param_1, param_2);
    float a = 1.0 - smoothstep(0.0, _63.blur, box);
    gl_FragData[0] = ((_63.color * _63.qt_Opacity) * a) * a;
}

