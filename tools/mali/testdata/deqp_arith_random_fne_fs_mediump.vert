attribute vec4 dEQP_Position;
attribute float a_d;
attribute vec4 a_af;
attribute float a_x;
attribute float a_k;
attribute vec2 a_ag;
varying mediump float d;
varying mediump vec4 af;
varying mediump float x;
varying mediump float k;
varying mediump vec2 ag;

void main()
{
	gl_Position = dEQP_Position;
	d = a_d;
	af = a_af;
	x = a_x;
	k = a_k;
	ag = a_ag;
}
