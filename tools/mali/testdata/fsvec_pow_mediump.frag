/* Fragment pow at mediump: a shape of docs/notes/mali-fs-vector-builtins.md,
 * the Mesa 26.2.1 drm-shim reference. */
precision mediump float;
varying vec4 v;
uniform vec4 u;
void main()
{
	gl_FragColor = vec4(pow(v.x, u.y), 0.0, 0.0, 1.0);
}
