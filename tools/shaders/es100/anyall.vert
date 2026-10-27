/* any()/all() over vectors narrower than four: a bvec2 from equal() on a
 * vec2 attribute (Qt's smoothtexture.vert outerEdge shape) and a bvec3 from
 * greaterThan() on a vec3 one. The reductions fold four lanes, so the
 * spare lanes must repeat a real one; folded as they came, all() reduced the
 * vertex fetch's spare (0, 1) and was false for every vertex. gl_test case
 * 86: v_color = (all(uv == 0), any(n > 0.5), 0, 1). */
attribute vec4 a_pos;
attribute vec2 a_uv;
attribute vec3 a_n;
varying vec4 v_color;
void main()
{
	float e = all(equal(a_uv, vec2(0.0))) ? 1.0 : 0.0;
	float f = any(greaterThan(a_n, vec3(0.5))) ? 1.0 : 0.0;
	v_color = vec4(e, f, 0.0, 1.0);
	gl_Position = a_pos;
}
