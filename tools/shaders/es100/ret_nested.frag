#version 100
/* dEQP functions.control_flow.return_in_nested_loop, fragment
 * (data/gles2/shaders/functions.test, Apache-2.0): a return from inside a
 * for inside a for. The board drew it wrong on the batch-16 build. */
precision mediump float;
varying mediump vec4 v_coords;
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
    gl_FragColor = vec4(func(v_coords.x), func(v_coords.y), 0.0, 1.0);
}
