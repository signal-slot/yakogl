#version 100
precision highp float;
precision highp int;
attribute highp vec4 dEQP_Position;
varying float v_out0;
bool out0;

void main()
{
    const bool cval = equal(ivec2 (1), ivec2 (2)).y;
    out0 = cval;
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
