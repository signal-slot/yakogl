/* dEQP-GLES2.functional.vertex_arrays.multiple_attributes.attribute_count.3,
 * rebuilt from VK-GL-CTS 93bca018 (Apache-2.0): modules/glshared/glsVertexArrayTests.cpp:843-974
 * (genVertexSource), ES 1.00 arm (:858-866), for the three vec2 arrays of
 * modules/gles2/functional/es2fVertexArrayTest.cpp:507-528. Shape C of
 * docs/notes/mali-deqp-varylink.md: v_color.z is the uniform u_colorScale
 * beside the attribute lanes x, y. */
attribute highp vec2 a_0;
attribute highp vec2 a_1;
attribute highp vec2 a_2;
uniform highp float u_coordScale;
uniform highp float u_colorScale;
varying mediump vec4 v_color;
void main(void)
{
	gl_PointSize = 1.0;
	highp vec2 coord = vec2(1.0, 1.0);
	highp vec3 color = vec3(1.0, 1.0, 1.0);
	coord = a_0.xy;
	color.rg = color.rg * a_1.xy;
	color.rg = color.rg * a_2.xy;
	v_color = vec4(u_colorScale * color, 1.0);
	gl_Position = vec4(u_coordScale * coord, 1.0, 1.0);
}
