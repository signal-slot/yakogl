attribute highp vec4 a_position;
attribute highp vec4 a_coords;
attribute mediump float a_one;
varying mediump vec3 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    gl_Position = a_position;
    mediump int one = int(a_one + 0.5);
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
    	mediump int i = 0;
	do  { res = res.yzwx; if (i == one*ui_one) break; }  while (++i < one*ui_three);

    v_color = res.rgb;
}
