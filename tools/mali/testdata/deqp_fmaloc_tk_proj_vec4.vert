attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
varying mediump vec4 v_texCoord;

void main()
{
	gl_Position = a_position;
	v_texCoord = a_in0;
}
