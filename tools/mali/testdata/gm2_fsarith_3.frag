/* Point light, Phong: normalize of a negation and of a quotient difference,
 * reflect, pow(max(0.0, dot()), constant) and max(0.0, dot()), at mediump. An
 * original reduction of the lit fragment shaders of glmark2's shading scene. */
precision mediump float;
varying vec3 vn;
varying vec4 vp;
uniform vec4 lpos;
uniform vec4 ldiff;
void main()
{
	vec3 eye = normalize(-vp.xyz);
	vec3 ld = normalize(lpos.xyz / lpos.w - vp.xyz / vp.w);
	vec3 nn = normalize(vn);
	vec3 r = reflect(-ld, nn);
	float st = pow(max(0.0, dot(r, eye)), 100.0);
	float dt = max(0.0, dot(nn, ld));
	vec4 res = vec4(0.8) * st + vec4(0.02, 0.02, 0.02, 1.0) + ldiff * dt;
	gl_FragColor = res;
}
