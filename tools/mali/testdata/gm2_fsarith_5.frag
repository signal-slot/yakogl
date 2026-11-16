/* Cel shading: length of a vector and of a difference, a quotient, normalize,
 * dot against a mix() of two constants, reflect and pow(max(0.0, dot()),
 * 10.0) in conditions, at mediump. An original reduction of the lit fragment
 * shader of glmark2's shading=cel. */
precision mediump float;
varying vec3 vn;
varying vec4 vp;
void main()
{
	vec4 base = vec4(0.0, 0.3, 0.0, 1.0);
	vec4 lsp = vec4(4.0, 3.0, 1.0, 1.0);
	vec4 col = base;
	vec3 vl = lsp.xyz - vp.xyz;
	vec3 N = normalize(vn);
	vec3 L = normalize(vl);
	float ndl = dot(N, L);
	float mx = max(ndl, 0.0);
	float att = length(lsp) / length(vl);
	if (att * mx >= 0.1) {
		col = vec4(0.0, 0.6, 0.0, 1.0);
	}
	vec3 V = normalize(-vp.xyz);
	if (dot(V, N) < mix(0.1, 0.4, mx)) {
		col = vec4(0.0, 0.0, 0.0, 1.0);
	}
	vec3 R = reflect(-L, N);
	float si = pow(max(0.0, dot(R, V)), 10.0);
	if (ndl > 0.0 && att * si > 0.5) {
		col = 0.7 * vec4(1.0) + 0.3 * col;
	}
	gl_FragColor = vec4(col.xyz, 1.0);
}
