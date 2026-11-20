precision mediump float;
attribute highp vec4 dEQP_Position;
attribute float in0;
varying float out0;


float func (float a)
{
    int i;
    for (i = 0; i < 6; i++) // negate a
    {
        a = -a;
        if (i == 4)
            a = -a;
    }

    for (; i < 10; i++) // keep a
    {
        if (i == 8)
            continue;
        else if (i == 9)
            break;
        a = -a;
    }

    return a;
}

void main()
{
    
    out0 = func(in0);
    gl_Position = dEQP_Position;

}
