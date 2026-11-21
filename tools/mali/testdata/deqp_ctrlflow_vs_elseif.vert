precision mediump float;
attribute highp vec4 dEQP_Position;
attribute float in0;
varying float out0;

void main()
{
    out0 = 0.0;
    if (in0 >= 2.0)
        out0 = 2.0;
    else if (in0 >= 1.0)
        out0 = 1.0;
    gl_Position = dEQP_Position;

}
