/* glmark2 terrain shape (18bc+4d26): three vec3, a vec2 and a vec3, in
 * that order, all from attributes and all read whole. */
attribute vec4 position;
attribute vec3 normal;
attribute vec3 tangent;
attribute vec2 uv;
uniform mat4 mvp;
varying vec3 vTangent;
varying vec3 vBinormal;
varying vec3 vNormal;
varying vec2 vUv;
varying vec3 vViewPosition;
void main()
{
	vViewPosition = position.xyz;
	vNormal = normal;
	vTangent = tangent;
	vBinormal = normal * tangent;
	vUv = uv;
	gl_Position = mvp * position;
}
