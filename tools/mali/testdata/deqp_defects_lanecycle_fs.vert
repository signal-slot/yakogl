/* dEQP-GLES2.functional.shaders.random.scalar_conversion.fragment.96: the vertex shader, verbatim from the .qpa
 * of deqp-gles2 (VK-GL-CTS 93bca018, Apache-2.0) run on Mesa 26.2.1's
 * drm-shim: on the Mali pair at f9b37651 the process dies before dEQP logs
 * it. The same fragment shader is .combined.96's. Defect `lanecycle`
 * of docs/notes/mali-deqp-defects.md. */
attribute vec4 dEQP_Position;
attribute float a_b;
varying mediump float b;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
}
