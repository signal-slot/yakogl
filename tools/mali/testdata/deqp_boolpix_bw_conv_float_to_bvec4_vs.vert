precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float in0;
varying vec4 v_out0;
bvec4 out0;


void main()
{
    
    out0 = bvec4(in0);
    gl_Position = dEQP_Position;
v_out0 = vec4(out0);

}
