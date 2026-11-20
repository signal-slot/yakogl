/* dEQP-GLES2.functional.shaders.random.conditionals.combined.75 (fragment shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape scalar_uniform of
 * docs/notes/mali-deqp-varylink2.md: a scalar varying of a uniform alone (a = 3.0 * n) beside interpolated varyings, refused by the vertex shader compiled alone. */
precision mediump float;
const bvec4 l = (bvec4(0.25, false, true, 0)).qtps;
const int k = -2;
vec4 j = vec4(float(5), float(k), true, float(true)).xwzy;
const bool i = float(-1.25) * float(-1.75) <= float(-1) - float(-3) + 0.001;
const float h = -0.5;
varying mediump float g;
varying mediump vec3 f;
const float e = (-3.375 - float(-3.0));
const float d = vec3(-9.75, false, 3).b * (-0.25);
const int b = 1 - int(0) * int(1);
varying mediump float a;
varying mediump float m;

void main()
{
	gl_FragColor = vec4(float(h), m, float(0.5), 0.5).sptq;
	{
		if ((l).x)
			j = vec4(k, h, float(0.5), 0.5).gbra;
		if (i)
			gl_FragColor = vec4(h, (g), f.g, e * -0.75);
		float c = d;
		gl_FragColor = (gl_FragColor * vec4(c, b, -0.0, a));
	}
}
