precision mediump float;
varying mediump vec4 k;
varying mediump vec4 j;
uniform samplerCube a;
varying mediump float b;
varying mediump float c;
varying mediump vec3 d;
varying mediump vec4 e;
const bool h = (bvec2(0.75, true)).s;
const bvec4 i = bvec4(h, true, 1.0, false).wzxy.ywxz;

void main()
{
	gl_FragColor = k.wyzx * j;
	int g = int((1));
	bvec3 f = i.rbga.zxy;
	f = bvec3(float(h), float(g), true).pst;
	f = (f);
	gl_FragColor = (e) - gl_FragColor;
	gl_FragColor = textureCube(a, d - vec3(0, c, b), 6.5);
}
