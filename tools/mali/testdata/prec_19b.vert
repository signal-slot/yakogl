attribute highp vec4 myVertex;
uniform mediump vec4 u4;
uniform mediump float u1;
uniform highp vec4 uh;
void main(void)
{
	gl_Position = myVertex * u1 + u4 + uh;
}
