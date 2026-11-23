precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec2 a_in0;
varying vec3 v_out0;
bvec3 out0;


void main()
{
    bvec2 in0 = bvec2(a_in0);

    out0 = in0.grg;
    gl_Position = dEQP_Position;
v_out0 = vec3(out0);

}
