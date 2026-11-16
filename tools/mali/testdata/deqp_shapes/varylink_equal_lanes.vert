attribute highp vec2 a_0;
uniform highp float u_coordScale;
uniform highp float u_colorScale;
varying mediump vec4 v_color;
void main(void)
{
	gl_PointSize = 1.0;
	highp vec2 coord = vec2(1.0, 1.0);
	highp vec3 color = vec3(1.0, 1.0, 1.0);
	coord = a_0.xy;
	v_color = vec4(u_colorScale * color, 1.0);
	gl_Position = vec4(u_coordScale * coord, 1.0, 1.0);
}
