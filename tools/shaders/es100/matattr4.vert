/* A mat4 vertex attribute: four consecutive attribute locations. */
attribute vec4 a_pos;
attribute mat4 a_xf;
varying vec4 v_color;
void main()
{
	vec4 rest = a_xf[0] + a_xf[1] + a_xf[2] + a_xf[3];
	float off = rest.y + rest.z + rest.w;
	v_color = vec4(a_xf[0][0], a_xf[1][0], a_xf[2][0], a_xf[3][0]);
	gl_Position = vec4(a_pos.xy + vec2(off), 0.0, 1.0);
}
