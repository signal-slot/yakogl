precision mediump float;
attribute highp vec4 dEQP_Position;
attribute float in0;
varying float out0;


float func (float a)
{
    for (int i = 0; i < 6; i++)
    {
        a = -a;
        for (int j = 0; j < 4; j++)
        {
            a = -a;
            if (i == 1)
                return a;
        }
        if (i == 4)
            return 1.0;
    }
    return 1.0;
}

void main()
{
    
    out0 = func(in0);
    gl_Position = dEQP_Position;

}
