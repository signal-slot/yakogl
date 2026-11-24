attribute highp vec4 a_position;
attribute mediump vec3 a_in0;
attribute mediump float a_in1;
varying mediump vec4 v_color;
uniform lowp samplerCube u_sampler;

void main()
{
	gl_Position = a_position;
	v_color = textureCubeLod(u_sampler, a_in0, a_in1);
}
