precision mediump float;
uniform vec4 u_k;
void main()
{
	float n = ceil(u_k.z * 3.0);
	float acc = 0.0;
	for (float i = 0.0; i < 8.0; i += 1.0) {
		if (i >= n) {
			break;
		}
		acc += u_k.y;
	}
	gl_FragColor = vec4(acc, 0.0, 0.0, 1.0);
}
