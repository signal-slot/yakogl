attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;
uniform mediump float uf_third;
uniform mediump float uf_one;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
	mediump float ndx = 0.05;
    while (ndx < uf_one)
    {
        res = res.yzwx;
		ndx += uf_third;
    }
    v_color = res.rgb;
}
