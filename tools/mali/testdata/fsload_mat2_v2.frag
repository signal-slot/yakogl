/* Packing order: a mat2 sorts as a vec4 (its four components), before
 * the vec2: m[0] in lanes 0-1, t in 2-3. */
precision mediump float;
varying vec2 t;
varying mat2 m;
void main() { gl_FragColor = vec4(m[0].y, t.x, 0.25, 1.0); }
