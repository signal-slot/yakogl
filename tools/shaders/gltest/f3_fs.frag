precision mediump float;
const vec4 c = (((vec4(1.625, -1.0, -1.5, 0.125)) * vec4(-1, 1.75, -1.0, true).wxyz));
const vec4 b = vec4(int(4) * int(1.0), float(-7.25) * 1.0 - -6.0, (float(3) * float(1.5)), (vec2(16, 2.75).y)) + vec4(2.75, -3, -3, -1).wyxz.grba.wzxy;
const vec4 a = vec4(vec4(true, -11, false, true).p, float(-11.5) - -7.0, float(-7.0) + float(8), float(-3.25) * float(0.5)).ragb * (vec4(-1, -1, 1.0, -1).rgba.qtps);

void main()
{
	gl_FragColor = (c.grba) * b - a;
}
