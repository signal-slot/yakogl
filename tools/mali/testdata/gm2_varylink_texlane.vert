/* glmark2 terrain shape (18bc+4d26): a varying written twice, its last
 * value from a vertex texture look-up at an attribute coordinate. */
attribute vec4 position;
attribute vec3 normal;
attribute vec2 uv;
uniform mat4 mvp;
uniform sampler2D tNormal;
varying vec3 vNormal;
varying vec2 vUv;
void main()
{
	vNormal = normal;
	vUv = uv;
	vNormal = texture2D(tNormal, uv).xyz;
	gl_Position = mvp * position;
}
