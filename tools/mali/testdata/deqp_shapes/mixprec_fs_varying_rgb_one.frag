varying mediump vec4 v_coords;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
	highp int ndx = 0;
    do
    {
        res = res.yzwx;
		ndx++;
    } while (ndx < 3);
    gl_FragColor = vec4(res.rgb, 1.0);
}
