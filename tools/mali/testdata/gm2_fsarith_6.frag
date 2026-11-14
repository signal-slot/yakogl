/* Normal-map lighting: a texel scaled to [-1, 1], a mat4 times a vec4 of it,
 * normalize, max(dot(), 0.0) and pow(max(dot(), 0.0), 100.0), at mediump. An
 * original reduction of the lit fragment shader of glmark2's bump scene. */
precision mediump float;
varying vec2 uv;
uniform sampler2D map;
uniform mat4 nm;
uniform vec3 light;
uniform vec3 half_v;
void main()
{
	vec3 raw = texture2D(map, uv).xyz;
	vec3 sc = raw * 2.0 - 1.0;
	vec3 N = normalize(vec3(nm * vec4(sc, 1.0)));
	vec3 L = normalize(light);
	vec3 H = normalize(half_v);
	vec4 diffuse = vec4(0.8) * max(dot(N, L), 0.0);
	vec4 spec = vec4(0.16) * pow(max(dot(N, H), 0.0), 100.0);
	gl_FragColor = vec4(0.1) + spec + diffuse;
}
