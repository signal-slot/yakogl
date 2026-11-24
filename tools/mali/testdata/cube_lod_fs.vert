#version 300 es
in highp vec4 a_position;
in mediump vec3 a_in0;
in mediump float a_in1;
out mediump vec3 v_texCoord;
out mediump float v_lodBias;

void main()
{
	gl_Position = a_position;
	v_texCoord = a_in0;
	v_lodBias = a_in1;
}
