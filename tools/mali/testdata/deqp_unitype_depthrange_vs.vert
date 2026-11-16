attribute highp vec4 a_position;
varying mediump vec4 v_color;
void main (void)
{
	v_color = vec4(gl_DepthRange.near, gl_DepthRange.far, gl_DepthRange.diff*0.5 + 0.5, 1.0);
	gl_Position = a_position;
}
