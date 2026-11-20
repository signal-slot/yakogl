attribute highp vec4 a_position;
attribute highp vec4 a_coords;
attribute mediump float a_one;
varying mediump vec3 v_color;
uniform highp float uf_third;
uniform highp float uf_one;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_coords;
    highp float one = a_one;
    mediump vec4 res = coords;
	highp float ndx = 0.05;
    do
    {
        res = res.yzwx;
		ndx += uf_third*one;
    } while (ndx < uf_one*one);
    v_color = res.rgb;
}
