varying mediump vec4 v_color;
varying mediump vec4 v_coords;
uniform sampler2D    ut_brick;
uniform mediump int  ui_one;
uniform mediump int  ui_two;

void main (void)
{
    gl_FragColor = v_color;
    for (int i = 0; i < ui_two; i++)
    {
        if (i > 0)
            if (texture2D(ut_brick, v_coords.xy*0.25+0.5).x < 0.7) discard;
    }
}
