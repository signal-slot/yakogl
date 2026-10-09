#version 300 es
precision highp float;
uniform highp uvec4 u_v;
layout(location = 0) out highp uvec4 o_v;
void main()
{
    // an integer target's output: the uniform's integers with the pixel's
    // column in alpha, written unconverted (gl_test case 196, intrt)
    o_v = uvec4(u_v.xyz, uint(gl_FragCoord.x));
}
