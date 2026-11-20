#version 100
precision highp float;
precision highp int;
attribute highp vec4 dEQP_Position;
varying float out0;

void main()
{
    const float cval = mod(float (87.65), float (3.7));
    out0 = cval;
    gl_Position = dEQP_Position;

}
