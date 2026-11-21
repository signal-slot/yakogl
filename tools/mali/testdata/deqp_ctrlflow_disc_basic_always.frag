varying mediump vec4 v_color;
varying mediump vec4 v_coords;
uniform sampler2D    ut_brick;
uniform mediump int  ui_one;

void main (void)
{
    gl_FragColor = v_color;
    discard;
}
