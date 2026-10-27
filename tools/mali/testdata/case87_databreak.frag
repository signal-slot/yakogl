#version 100
precision highp float;
varying vec4 v_color;
void main()
{
	float s = v_color.w;
	float top = v_color.y;
	float count = 0.0;
	for (int i = 0; i < 8; i++) {
		if (float(i) >= s * 8.0) {
			break;
		}
		if (top > 0.5 && i == 2) {
			continue;
		}
		count += 1.0;
	}
	gl_FragColor = vec4(count / 8.0, 0.0, top > 0.5 ? 1.0 : 0.0, 1.0);
}
