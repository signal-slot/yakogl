#version 100
precision highp float;
precision highp int;
attribute highp vec4 dEQP_Position;
varying vec2 out0;

void main()
{
    const bvec2 cval = greaterThan(ivec2 (1), ivec2 (2));
    out0 = vec2(cval);
    gl_Position = dEQP_Position;

}
