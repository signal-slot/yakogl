/* dEQP-GLES2.functional.shaders.random.scalar_conversion.fragment.96: the fragment shader, verbatim from the .qpa
 * of deqp-gles2 (VK-GL-CTS 93bca018, Apache-2.0) run on Mesa 26.2.1's
 * drm-shim: on the Mali pair at f9b37651 the process dies before dEQP logs
 * it. The same fragment shader is .combined.96's. Defect `lanecycle`
 * of docs/notes/mali-deqp-defects.md. */
precision mediump float;
const float h = ((float(-0.0)));
const bool a = false;
varying mediump float b;
const int c = int(float(int(0.0)));
const int d = (int(float(c)));
const float e = float((0.5));
const bool f = true;
float g = float(2.625) * -1.0 * float(float(-2.0));

void main()
{
	gl_FragColor = vec4((float(-1.0)), -0.25 + h, -0.25, (b));
	gl_FragColor = vec4(-1.0, g, f, e) * gl_FragColor * (vec4(d, c, b, a));
	gl_FragColor = gl_FragColor;
}
