#version 100
/* LOCALS (original reduction of glmark2's effect2d 5x3 box kernel shape): 15
 * texture look-ups, each with its own vec2 coordinate local and vec4 sample
 * local (30 locals), plus the step, the weight and the accumulator: 33. */
precision mediump float;
uniform sampler2D Texture0;
uniform vec2 px;
varying vec2 TextureCoord;
void main()
{
	vec2 step = px;
	float w = 1.0 / 15.0;
	vec4 result = vec4(0.0);
	vec2 c0 = TextureCoord + step * vec2(-2.0, -1.0);
	vec4 s0 = texture2D(Texture0, c0);
	result += s0 * w;
	vec2 c1 = TextureCoord + step * vec2(-1.0, -1.0);
	vec4 s1 = texture2D(Texture0, c1);
	result += s1 * w;
	vec2 c2 = TextureCoord + step * vec2(0.0, -1.0);
	vec4 s2 = texture2D(Texture0, c2);
	result += s2 * w;
	vec2 c3 = TextureCoord + step * vec2(1.0, -1.0);
	vec4 s3 = texture2D(Texture0, c3);
	result += s3 * w;
	vec2 c4 = TextureCoord + step * vec2(2.0, -1.0);
	vec4 s4 = texture2D(Texture0, c4);
	result += s4 * w;
	vec2 c5 = TextureCoord + step * vec2(-2.0, 0.0);
	vec4 s5 = texture2D(Texture0, c5);
	result += s5 * w;
	vec2 c6 = TextureCoord + step * vec2(-1.0, 0.0);
	vec4 s6 = texture2D(Texture0, c6);
	result += s6 * w;
	vec2 c7 = TextureCoord + step * vec2(0.0, 0.0);
	vec4 s7 = texture2D(Texture0, c7);
	result += s7 * w;
	vec2 c8 = TextureCoord + step * vec2(1.0, 0.0);
	vec4 s8 = texture2D(Texture0, c8);
	result += s8 * w;
	vec2 c9 = TextureCoord + step * vec2(2.0, 0.0);
	vec4 s9 = texture2D(Texture0, c9);
	result += s9 * w;
	vec2 c10 = TextureCoord + step * vec2(-2.0, 1.0);
	vec4 s10 = texture2D(Texture0, c10);
	result += s10 * w;
	vec2 c11 = TextureCoord + step * vec2(-1.0, 1.0);
	vec4 s11 = texture2D(Texture0, c11);
	result += s11 * w;
	vec2 c12 = TextureCoord + step * vec2(0.0, 1.0);
	vec4 s12 = texture2D(Texture0, c12);
	result += s12 * w;
	vec2 c13 = TextureCoord + step * vec2(1.0, 1.0);
	vec4 s13 = texture2D(Texture0, c13);
	result += s13 * w;
	vec2 c14 = TextureCoord + step * vec2(2.0, 1.0);
	vec4 s14 = texture2D(Texture0, c14);
	result += s14 * w;
	gl_FragColor = vec4(result.xyz, 1.0);
}
