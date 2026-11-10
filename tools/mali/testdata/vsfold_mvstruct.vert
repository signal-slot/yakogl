/* Qt Quick 3D's motionvector shader's shape in GLSL ES 1.00: a uniform
 * struct of six mat4s (Qt flattens its uniform buffer into one) read as
 * `ubuf.mvp * v` and `ubuf.prevMVP * v`. The GC7000 online path computed
 * gl_Position wrong here while the IR was right (frontend2's ES 3.00 step 1,
 * motionvector_300.vert; this twin differs identically). */
struct buf {
	mat4 mvp;
	mat4 prevMVP;
	mat4 instanceLocal;
	mat4 instanceGlobal;
	mat4 prevInstanceLocal;
	mat4 prevInstanceGlobal;
	vec4 currentAndLastJitter;
	float velocityAmount;
	int morphTargetCount;
	vec2 padding;
};
uniform buf ubuf;
attribute vec3 attr_pos;
varying vec4 vPosition;
varying vec4 vPrevPosition;
void main()
{
	vec4 vp = vec4(attr_pos, 1.0);
	vec4 _41 = ubuf.mvp * vp;
	vPosition = _41;
	gl_Position = _41;
	vPrevPosition = ubuf.prevMVP * vec4(attr_pos, 1.0);
}
