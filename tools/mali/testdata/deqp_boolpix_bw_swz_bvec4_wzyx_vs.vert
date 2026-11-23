precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec4 a_in0;
varying vec4 v_out0;
bvec4 out0;


void main()
{
    bvec4 in0 = bvec4(a_in0);

    out0 = in0.wzyx;
    gl_Position = dEQP_Position;
v_out0 = vec4(out0);

}
