/* Directional light, Blinn-Phong: normalize of a uniform and of a mat4 times a
 * varying, max(dot(), 0.0) and pow(max(dot(), 0.0), constant), at highp. An
 * original reduction of the lit fragment shaders of glmark2's shading/bump
 * scenes. */
precision highp float;
varying vec3 n;
varying vec2 uv;
uniform vec3 light;
uniform vec3 half_v;
uniform mat4 nm;
void main()
{
	vec3 N = normalize(vec3(nm * vec4(n, 1.0)));
	vec3 L = normalize(light);
	vec3 H = normalize(half_v);
	float d = max(dot(N, L), 0.0);
	float s = pow(max(dot(N, H), 0.0), 100.0);
	gl_FragColor = vec4(vec3(0.1) + vec3(0.8 * d) + vec3(0.2 * s), 1.0);
}
