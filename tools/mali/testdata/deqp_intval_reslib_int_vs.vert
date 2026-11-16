precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float in0;
varying float v_out0;
int out0;


void main()
{
    
    out0 = int(in0);
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
