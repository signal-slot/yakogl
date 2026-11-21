#version 100
precision highp float;
precision highp int;
attribute highp vec4 dEQP_Position;
varying float v_out0;
bool out0;

void main()
{
    const bool cval = equal(bvec2 (true), bvec2 (false)).y;
    out0 = cval;
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
