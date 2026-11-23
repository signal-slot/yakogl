precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec2 a_in0;
varying float v_out0;
bool out0;


void main()
{
    bvec2 in0 = bvec2(a_in0);

    out0 = in0.r;
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
