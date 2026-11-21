precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec4 a_in0;
varying vec4 v_out0;
vec4 out0;

void main()
{
    vec4 in0 = a_in0;
    out0 = in0.xyzw;
    gl_Position = dEQP_Position;
    v_out0 = out0;
}
