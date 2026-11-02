#version 100
/* dEQP functions.control_flow.return_in_nested_loop, vertex
 * (data/gles2/shaders/functions.test, Apache-2.0). es100/va.frag draws
 * v_color. */
precision mediump float;
attribute vec4 a_position;
attribute vec4 a_coords;
varying mediump vec4 v_color;
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
    gl_Position = a_position;
    v_color = vec4(func(a_coords.x), func(a_coords.y), 0.0, 1.0);
}
