/* dEQP-GLES2.functional.shaders.random.basic_expression.vertex.27 (vertex shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape vec_uniform of
 * docs/notes/mali-deqp-varylink2.md: a uniform scalar lane in a vec4 constructor beside attribute lanes, refused by the vertex shader compiled alone. */
attribute vec4 dEQP_Position;
attribute float i;
const float h = -2.0;
uniform mediump float g;
attribute float f;
vec4 a = vec4(6.625, -19.125, 2.875, h) * vec4(1.25, 0.5, -1.5, -0.5) - vec4(1.125, 2.0, h, 4.625) * vec4(h, 0.25, 0.5, 1.25);
attribute float b;
const float c = 7.0 * -1.0 + (float(6.0));
float d = (float(-45.125)) + c;
attribute float e;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(float(i) + h, g, (float(f)), (float(b)));
	a = a;
	a = vec4(e, b, float(d), b) * (vec4(c, b, 0.25, -2.25));
}
