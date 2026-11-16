#version 100
precision highp float;
precision highp int;
attribute highp vec4 dEQP_Position;
varying float out0;

void main()
{
    const float cval = floor(float (37.3));
    out0 = cval;
    gl_Position = dEQP_Position;

}
