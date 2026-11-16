/* dEQP-GLES2.functional.draw.draw_arrays.triangles.multiple_attributes, rebuilt
 * from VK-GL-CTS 93bca018 (Apache-2.0): modules/glshared/glsDrawTest.cpp:1698-1807
 * (genVertexSource), ES 1.00 arm (:1837-1846), for the vec2 position array and
 * the vec2 colour array of modules/gles2/functional/es2fDrawTests.cpp:200-220.
 * Shape C of docs/notes/mali-deqp-varylink.md: v_color.z is a uniform lane
 * beside the attribute lanes x, y. */
attribute highp vec2 a_0;
attribute highp vec2 a_1;
uniform highp float u_coordScale;
uniform highp float u_colorScale;
varying mediump vec4 v_color;
void main(void)
{
	gl_PointSize = 1.0;
	highp vec2 coord = vec2(0.0, 0.0);
	highp vec3 color = vec3(1.0, 1.0, 1.0);
	coord += vec2(a_0.xy);
	color.rg = color.rg * vec2(a_1.xy);
	v_color = vec4(u_colorScale * color, 1.0) * 0.5 + vec4(0.5, 0.5, 0.5, 0.5);
	gl_Position = vec4(u_coordScale * coord, 1.0, 1.0);
}
