attribute highp vec4 a_position;
varying mediump vec2 v;
void main()
{
	gl_Position = a_position;
	v = a_position.xy;
}
