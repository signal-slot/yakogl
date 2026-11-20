attribute highp vec4 a_position;
attribute mediump vec3 a_in0;
attribute mediump float a_in1;
varying mediump vec3 v_texCoord;
varying mediump float v_lodBias;

void main()
{
	gl_Position = a_position;
	v_texCoord = a_in0;
	v_lodBias = a_in1;
}
