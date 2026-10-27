/* dEQP-GLES2.functional.vertex_arrays, generated as by VK-GL-CTS
 * modules/glshared/glsVertexArrayTests.cpp genVertexSource (Apache-2.0),
 * ES 1.00 arm, for two vec4 arrays (multiple_attributes.attribute_count.2). It writes gl_PointSize,
 * which the V3D drops for a draw that is not points. */
attribute highp vec4 a_0;
attribute highp vec4 a_1;
uniform highp float u_coordScale;
uniform highp float u_colorScale;
varying mediump vec4 v_color;
void main(void)
{
	gl_PointSize = 1.0;
	highp vec2 coord = vec2(1.0, 1.0);
	highp vec3 color = vec3(1.0, 1.0, 1.0);
	coord = a_0.xy;
	coord += a_0.zw;
	color = color.rgb * a_1.xyz * a_1.w;
	v_color = vec4(u_colorScale * color, 1.0);
	gl_Position = vec4(u_coordScale * coord, 1.0, 1.0);
}
