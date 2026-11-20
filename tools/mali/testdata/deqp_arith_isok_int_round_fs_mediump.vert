precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec2 a_in0;
varying vec4 v_out0;
ivec4 out0;


void main()
{
    ivec2 in0 = ivec2(a_in0);

    out0 = in0.gggg;
    gl_Position = dEQP_Position;
v_out0 = vec4(out0);

}
