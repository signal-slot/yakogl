attribute vec4 dEQP_Position;
attribute float a_b;
attribute float a_d;
attribute float a_i;
attribute float a_o;
attribute float a_p;
attribute vec4 a_t;
attribute vec4 a_u;
varying mediump float b;
varying mediump float d;
varying mediump float i;
varying mediump float o;
varying mediump float p;
varying mediump vec4 t;
varying mediump vec4 u;

void main()
{
	gl_Position = dEQP_Position;
	b = a_b;
	d = a_d;
	i = a_i;
	o = a_o;
	p = a_p;
	t = a_t;
	u = a_u;
}
