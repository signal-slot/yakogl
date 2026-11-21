varying mediump vec4 v_color;
varying mediump vec4 v_coords;
uniform sampler2D    ut_brick;
uniform mediump int  ui_one;

void myfunc (void)
{
    for (int i = 0; i < 2; i++)
    {
        if (i > 0)
            discard;
    }
}

void main (void)
{
    gl_FragColor = v_color;
    myfunc();
}
