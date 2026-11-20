precision mediump float;
attribute highp vec4 dEQP_Position;
attribute float in0;
varying float out0;


float func (float a)
{
    for (int i = 0; i < 6; i++)
    {
        if (i == 0)
            continue;
        else if (i == 1)
        {
        }
        else if (i == 3)
            break;
        else
            return a;
        a = -a;
    }

    return 1.0;
}

void main()
{
    
    out0 = func(in0);
    gl_Position = dEQP_Position;

}
