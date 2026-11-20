precision mediump float;
attribute highp vec4 dEQP_Position;
attribute float in0;
varying float out0;


float func (float a)
{
    return -a;
}

float func (float a, float b)
{
    return a * b;
}

void main()
{
    out0 = func(in0) * func(-0.5, -2.0);
    gl_Position = dEQP_Position;

}
