#version 100
precision highp float;
precision highp int;
attribute highp vec4 dEQP_Position;
varying float out0;

void main()
{
    const float cval = sign(float (-18.0));
    out0 = cval;
    gl_Position = dEQP_Position;

}
