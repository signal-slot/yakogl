precision mediump float;
precision mediump int;

attribute highp vec4 dEQP_Position;
attribute float a_in0;
varying mat3 out0;

void main()
{
    bool in0 = bool(a_in0);

    out0 = mat3(in0);
    gl_Position = dEQP_Position;

}
