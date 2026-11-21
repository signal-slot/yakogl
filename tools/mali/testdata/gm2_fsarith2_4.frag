/* A distance attenuation: the length of a highp vector divided by a mediump
 * uniform, clamped to one and taken from one (the `lDistance` of glmark2's
 * terrain lighting, fs 4d26). An original reduction. */
precision mediump float;
uniform float dist;
varying highp vec3 lv;
void main()
{
	float f = 1.0 - min(length(lv) / dist, 1.0);
	gl_FragColor = vec4(f, f, f, 1.0);
}
