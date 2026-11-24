attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
    	mediump int i = 0;
    while(i++ < 3) { if (i == 1) continue; res = res.yzwx; continue; }
    v_color = res.rgb;
}
