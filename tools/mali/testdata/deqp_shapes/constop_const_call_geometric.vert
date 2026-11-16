#version 100
precision highp float;
precision highp int;
attribute highp vec4 dEQP_Position;
varying float out0;

void main()
{
    const float cval = distance(vec2(1.0), vec2(2.0));
    out0 = cval;
    gl_Position = dEQP_Position;

}
