/* A mat2 vertex attribute: two consecutive attribute locations, one per
 * column. The colour is the matrix's first row, so a column that lands in
 * the wrong register shows as the wrong colour; the rest of the matrix moves
 * the quad, so no element is dead. */
attribute vec4 a_pos;
attribute mat2 a_xf;
varying vec4 v_color;
void main()
{
	float off = a_xf[0][1] + a_xf[1][1];
	v_color = vec4(a_xf[0][0], a_xf[1][0], 0.5, 1.0);
	gl_Position = vec4(a_pos.xy + vec2(off), 0.0, 1.0);
}
