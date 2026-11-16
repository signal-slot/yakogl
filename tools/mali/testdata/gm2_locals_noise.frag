#version 100
/* LOCALS (original reduction of the terrain noise shape): a chain of vec3 and
 * vec4 temporaries, each built from the earlier ones with dot, *, +, abs, min
 * and max only (no floor/mod/step: one refusal family at a time). 36 locals
 * by declaration. */
precision highp float;
uniform vec4 m;
varying vec3 vUv;
void main()
{
	vec3 x0 = vUv + m.xyz;
	vec3 x1 = x0 * m.w;
	vec3 x2 = x1 + x0;
	vec3 x3 = abs(x2 - x1);
	vec3 g = min(x0, x1);
	vec3 l = max(x2, x3);
	vec3 i1 = min(g, l);
	vec3 i2 = max(g, l);
	vec4 p = vec4(dot(x0, i1), dot(x1, i2), dot(x2, g), dot(x3, l));
	vec4 q = p * p + m;
	vec4 r = abs(q) + p;
	vec4 j = r * m.x + q;
	vec4 x_ = j * m.y;
	vec4 y_ = j - x_;
	vec4 x = x_ * m.z + y_;
	vec4 y = y_ * m.z + x_;
	vec4 h = vec4(1.0) - abs(x) - abs(y);
	vec4 b0 = vec4(x.xy, y.xy);
	vec4 b1 = vec4(x.zw, y.zw);
	vec4 s0 = b0 * 2.0 + h;
	vec4 s1 = b1 * 2.0 + h;
	vec4 a0 = b0.xzyw + s0.xzyw * h.xxyy;
	vec4 a1 = b1.xzyw + s1.xzyw * h.zzww;
	vec3 p0 = vec3(a0.xy, h.x);
	vec3 p1 = vec3(a0.zw, h.y);
	vec3 p2 = vec3(a1.xy, h.z);
	vec3 p3 = vec3(a1.zw, h.w);
	vec4 norm = vec4(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)) * m;
	vec3 u0 = p0 * norm.x;
	vec3 u1 = p1 * norm.y;
	vec3 u2 = p2 * norm.z;
	vec3 u3 = p3 * norm.w;
	vec4 d = max(m - vec4(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), vec4(0.0));
	d = d * d;
	vec4 e = vec4(dot(u0, x0), dot(u1, x1), dot(u2, x2), dot(u3, x3));
	float n = dot(d * d, e);
	float na = abs(n);
	gl_FragColor = vec4(na, na, na, 1.0);
}
