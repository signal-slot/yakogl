#version 300 es
precision highp float;
precision highp int;
uniform int i;
uniform vec4 a;
out vec4 colour;
void main()
{
    vec4 t[4];
    t[0] = a;
    t[1] = a.yzwx;
    t[2] = a.zwxy;
    t[3] = a.wxyz;
    colour = t[i & 3];
}
