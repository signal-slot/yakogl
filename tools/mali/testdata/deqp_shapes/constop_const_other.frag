varying mediump vec4 v_coords;
uniform highp float uf_one;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
	highp float ndx = 0.05;
    do
    {
        res = res.yzwx;
		ndx += 0.333333;
    } while (ndx < 1.0);
    gl_FragColor = vec4(res.rgb, 1.0);
}
