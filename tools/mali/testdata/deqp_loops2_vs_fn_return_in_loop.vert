precision mediump float;
attribute highp vec4 dEQP_Position;
attribute float in0;
varying float out0;


float func (float a)
{
    for (int i = 0; i < 1; i++)
        return -a;
    return 1.0;
}

void main()
{
    
    out0 = func(in0);
    gl_Position = dEQP_Position;

}
