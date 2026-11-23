precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute vec2 in0;
attribute vec2 a_in1;
varying vec4 v_out0;
bvec4 out0;


void main()
{
    bvec2 in1 = bvec2(a_in1);

    out0 = bvec4(in0, in1);
    gl_Position = dEQP_Position;
v_out0 = vec4(out0);

}
