varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    	mediump int i = 0;
	do 
    {
        res = coords.wxyz;
        mediump int j = 0;
        while (j++ < ui_two)
            res = res.yzwx;
        coords = res;
    }  while (++i < ui_three);

    gl_FragColor = vec4(res.rgb, 1.0);
}
