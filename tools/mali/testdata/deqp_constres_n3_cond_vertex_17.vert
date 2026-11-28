attribute vec4 dEQP_Position;
int a = 3 * 1 - 6 * 1;
attribute float b;
attribute float c;
float d = 3.125 * float(1.25) - (float(3.5));
uniform mediump float e;
int f = ivec4(-7.0, 5.5, 1.0, 11).z - int(float(2.0));
const ivec4 g = ivec4(float(float(7.25)), float(3) + 0.001 >= float(-1.5), (float(-2.25)), (int(9)));
const bool h = true;
int i = int((2));
const bool j = int(int(4.0)) > (int(6.25));
const bool k = h;
attribute float l;
int m = int(float(1.0) * float(1));
attribute vec4 n;
varying mediump vec4 v_color;

void main()
{
	gl_Position = dEQP_Position;
	v_color = vec4(-3.0, -11.0, -7.25, 6.5).tqps + vec4(1.0, -5.0, 7, -2.5).barg;
	v_color = vec4(4, i, j, -13.5) - n - v_color;
	if (vec3(b, 0, -1.25) * vec3(f, m, l) != vec3(int(k), int(j), int(i)))
		if (2 * 1 != (int(h)))
			f = (g).z;
		else 
			v_color = vec4(true, (e), float(d) + 0.001 >= c, int(b) >= a);
}
