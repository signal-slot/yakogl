precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute vec3 a_in0;
attribute vec3 a_in1;
attribute vec3 a_in2;
varying mat3 out0;

void main()
{
    ivec3 in0 = ivec3(a_in0);
ivec3 in1 = ivec3(a_in1);
ivec3 in2 = ivec3(a_in2);

    out0 = mat3(in0, in1, in2);
    gl_Position = dEQP_Position;

}
