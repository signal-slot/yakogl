precision mediump float;
attribute highp vec4 dEQP_Position;
attribute float in0;
varying float out0;
varying float out1;

void main()
{
    out0 = 0.0;
    out1 = 1.0;
    if (in0 >= 1.0)
    {
        out0 = 1.0;
        out1 = 0.0;
    }
    gl_Position = dEQP_Position;

}
