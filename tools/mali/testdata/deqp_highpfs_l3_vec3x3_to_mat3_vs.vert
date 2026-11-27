precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute vec3 in0;
attribute vec3 in1;
attribute vec3 in2;
varying mat3 out0;

void main()
{
    
    out0 = mat3(in0, in1, in2);
    gl_Position = dEQP_Position;

}
