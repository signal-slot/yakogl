attribute vec4 dEQP_Position;
uniform mediump int a;
const int b = int(-3) * int(true) + (int(3.0));
attribute float c;
uniform mediump int d;
attribute float e;
const int f = 7 + (int(-8));
const float g = 9.5 - 4.0 + vec3(14, 15, -4.5).p;
const float h = (float(-10.5)) - (-4.5);
const float j = float(-6.75 - float(-8.0));
const float k = vec4(j, -4.5, -3.0, h).g - vec2(-3.5, true).x;
const float l = float(11.0) * float(-0.5) - float(-1.0) * -1.0;
const vec4 m = vec4(1.5, float(-14.625), float(-5.5), float(-5.5)) * (vec4(true, k, true, 1.75));
uniform sampler2D n;
uniform samplerCube o;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(false, false, 2.75, true).pqst.ragb;
	v_color = textureCube(o, v_color.wzx);
	v_color = vec4(true, int(d), float(2.38419e-07), float(14.1)).gbar;
	v_color = v_color;
	vec4 i = texture2DLod(n, vec2(h, 3.25) * vec2(d, false), e - k);
	i = (m) + vec4(-6.5, l, 1, -5.5) * vec4(k, 1.0, j, -1.0);
	v_color = i.wzxy + vec4(-4, 4.5, h, 4) * vec4(-1.0, 1.0, g, -2.0);
	v_color = v_color * vec4(1, f, -1.5, e) * (vec4(d, c, b, a));
}
