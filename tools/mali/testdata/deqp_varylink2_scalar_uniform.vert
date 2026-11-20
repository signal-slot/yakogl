/* dEQP-GLES2.functional.shaders.random.conditionals.combined.75 (vertex shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape scalar_uniform of
 * docs/notes/mali-deqp-varylink2.md: a scalar varying of a uniform alone (a = 3.0 * n) beside interpolated varyings, refused by the vertex shader compiled alone. */
attribute vec4 dEQP_Position;
attribute float p;
attribute vec4 o;
uniform mediump float n;
varying mediump float g;
varying mediump vec3 f;
varying mediump float a;
varying mediump float m;

void main()
{
	gl_Position = dEQP_Position;
	g = p;
	f = (o).yzw;
	a = float(int(3)) * float(n);
	m = ((float(-1.75)));
}
