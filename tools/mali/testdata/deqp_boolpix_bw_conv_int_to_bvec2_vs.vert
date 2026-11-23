precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float a_in0;
varying vec2 v_out0;
bvec2 out0;


void main()
{
    int in0 = int(a_in0);

    out0 = bvec2(in0);
    gl_Position = dEQP_Position;
v_out0 = vec2(out0);

}
