#version 100
precision highp float;
precision highp int;
attribute highp vec4 dEQP_Position;
varying float out0;

void main()
{
    const float cval = mod(vec4 (87.65), vec4 (3.7)).w;
    out0 = cval;
    gl_Position = dEQP_Position;

}
