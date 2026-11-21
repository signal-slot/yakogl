/* floor of a mediump vec3 and step between its swizzles: the first corner and
 * the simplex ordering of the 3D noise of glmark2's terrain height pass (fs
 * 1637). An original reduction. */
precision mediump float;
varying vec3 v;
void main()
{
	const vec2 C = vec2(1.0 / 6.0, 1.0 / 3.0);
	vec3 i = floor(v + dot(v, C.yyy));
	vec3 x0 = v - i + dot(i, C.xxx);
	vec3 g = step(x0.yzx, x0.xyz);
	vec3 i1 = min(g, 1.0 - g.zxy);
	gl_FragColor = vec4(x0 + i1, 1.0);
}
