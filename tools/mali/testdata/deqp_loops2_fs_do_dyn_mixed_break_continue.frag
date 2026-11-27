varying mediump vec4 v_coords;
varying mediump float v_one;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    mediump int one = int(v_one + 0.5);
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    	mediump int i = 0;
	do  { if (i == 0) continue; else if (i == 3) break; res = res.yzwx; }  while (++i < one*ui_five);

    gl_FragColor = vec4(res.rgb, 1.0);
}
