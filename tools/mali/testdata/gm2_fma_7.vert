/* mix(vec4, vec4, float): a + (b - a) * t, a fused multiply-add per lane. */
attribute vec4 a_pos;
attribute vec4 aP;
attribute vec4 aC;
uniform float k;
varying vec4 v_c;
void main()
{
	v_c = mix(aP, aC, k);
	gl_Position = a_pos;
}
