attribute highp vec4 pos;
uniform mediump vec4 offs;
uniform highp vec4 scale;
void main(void)
{
	gl_Position = pos * scale + offs;
}
