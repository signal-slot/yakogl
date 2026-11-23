precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute vec4 a_in0;
varying float v_out0;
bool out0;


void main()
{
    bvec4 in0 = bvec4(a_in0);

    out0 = bool(in0);
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
