#version 100
precision highp float;
precision highp int;
attribute highp vec4 dEQP_Position;
varying float out0;

void main()
{
    const float cval = asin(vec2 (0.0)).y;
    out0 = cval;
    gl_Position = dEQP_Position;

}
