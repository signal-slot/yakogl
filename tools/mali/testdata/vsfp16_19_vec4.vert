attribute highp vec4 pos;
uniform mediump vec4 offs;
void main(void)
{
	gl_Position = pos + offs;
}
