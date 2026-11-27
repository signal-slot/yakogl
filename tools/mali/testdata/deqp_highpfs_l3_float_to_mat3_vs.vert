precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float in0;
varying mat3 out0;

void main()
{
    
    out0 = mat3(in0);
    gl_Position = dEQP_Position;

}
