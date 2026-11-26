#version 100
attribute highp vec4 a_position;
void main()
{
	gl_Position = a_position / 2.0;	// a \ b
}
