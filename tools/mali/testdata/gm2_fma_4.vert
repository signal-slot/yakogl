/* jellyfish's offset, the first refusal of its vertex shader once the sin is
 * left out: smoothstep(0.0, 1.0, max(0.0, -y - 0.8) / 10.0), then its use
 * in (1.0 - offset) * (P + C / 12.0 * k). */
attribute vec4 a_pos;
attribute vec3 aCol;
uniform float k;
varying vec4 v_c;
void main()
{
	float offset = smoothstep(0.0, 1.0, max(0.0, -a_pos.y - 0.8) / 10.0);
	vec3 pos = a_pos.xyz + aCol / 12.0 * k * (1.0 - offset);
	v_c = vec4(pos, 1.0);
	gl_Position = a_pos;
}
