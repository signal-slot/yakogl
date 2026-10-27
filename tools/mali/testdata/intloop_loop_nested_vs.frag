/* dEQP-GLES2.functional.vertex_arrays, genFragmentSource (glsVertexArrayTests.cpp,
 * Apache-2.0), ES 1.00 arm: the pass-through fragment shader. */
varying mediump vec4 v_color;
void main(void)
{
	gl_FragColor = v_color;
}
