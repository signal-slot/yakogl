varying mediump vec4 v_coords;
uniform mediump int ui_three;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
	mediump int ndx = 0;
    while (ndx < ui_three)
    {
        res = res.yzwx;
		ndx++;
    }
    gl_FragColor = vec4(res.rgb, 1.0);
}
