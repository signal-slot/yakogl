/* dEQP-GLES2.functional.shaders.random.all_features.vertex.10: the vertex shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape constglobal of
 * docs/notes/mali-deqp-negzero.md. */
attribute vec4 dEQP_Position;
attribute vec4 q;
attribute float a;
const int b = -7 + (int(8.0) - int(false)) * ivec4(-12, float(-5.0), bool(true), bool(1.0)).z;
const bool c = asin((vec3(false, 0.75, b)).t) < (float(4) * float(-0.25) * vec2(b, -2.0).t);
uniform mediump float d;
attribute float e;
const float g = (0.5) - (5.5) + -2.5 + 3.5;
bvec3 h = bvec3((float(-1)) * -0.0, int(int(-1)) + ivec3(g, b, b).b == int(6 - int(7.5)), float(float(0.25) - float(-4.25)) + (g * float(1.0)));
const bvec3 i = bvec3(vec3(6.0, -1.75, 4.75) + vec3(-6, 5.0, -2.5) - vec3(1.0, 4.0, 3.0).pst != (vec3(float(-1.75), 5, float(2.25))), sqrt(inversesqrt(float(-1) * -1.0)), c);
const bool j = vec4(5.0, -8, 8.0, g).pst.tsp + vec4(-6, -0.75, 3, -2).sqpt.tsp != (vec3(float(3.25), float(g), c)) * vec3(-1.5, -1, 1).grb.pts;
attribute float k;
const float l = ((float(float(6.5) - 5.0)));
const bvec3 m = (bvec4(float(b), c, float(0.5), b).xzwy).tqp;
attribute vec2 n;
const ivec3 o = (((ivec3(b, 1, true))) + (ivec3(c, j, -7)).spt);
float p = (vec2(13, b)).gr.ts.x;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = tan(sin(q));
	int f = int((int(p))) + o.y;
	if ((abs(log2((float(k) + float(j))) - (exp((n).s))) < 0.001) == true)
		h = m.yzx.stp;
	if (inversesqrt(float(-1.375) + l + 0.0) + 0.001 >= log(k - float(j)))
		if (c)
			h = i;
		else 
			v_color = vec4(int(int(-9.0) - int(g)), -1.0, f, float(5)) - vec4(e, int(float(d)) * 1, 6 * int(c) + (b), a);
}
