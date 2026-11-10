/* Fragment nconst at highp: a shape of docs/notes/mali-fs-vector-builtins.md,
 * the Mesa 26.2.1 drm-shim reference. */
precision highp float;
varying vec4 v;
uniform vec4 u;
void main()
{
	gl_FragColor = vec4(normalize(vec3(1.0, 2.0, 2.0)) * v.x, 1.0);
}
