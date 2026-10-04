/* A mat3 vertex attribute: three consecutive attribute locations. */
attribute vec4 a_pos;
attribute mat3 a_xf;
varying vec4 v_color;
void main()
{
	float off = a_xf[0][1] + a_xf[1][1] + a_xf[2][1] + a_xf[0][2] + a_xf[1][2] + a_xf[2][2];
	v_color = vec4(a_xf[0][0], a_xf[1][0], a_xf[2][0], 1.0);
	gl_Position = vec4(a_pos.xy + vec2(off), 0.0, 1.0);
}
