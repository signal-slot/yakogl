/* Fragment fract at highp: a shape of docs/notes/mali-fs-vector-builtins.md,
 * the Mesa 26.2.1 drm-shim reference. */
precision highp float;
varying vec4 v;
uniform vec4 u;
void main()
{
	gl_FragColor = vec4(fract(v.x), 0.0, 0.0, 1.0);
}
