varying mediump vec4 v_coords;
uniform mediump float uf_third;
uniform mediump float uf_one;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
	mediump float ndx = 0.05;
    do
    {
        res = res.yzwx;
		ndx += uf_third;
    } while (ndx < uf_one);
    gl_FragColor = vec4(res.rgb, 1.0);
}
