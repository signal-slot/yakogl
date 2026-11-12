/* + vec2(0.0, -0.0): zeros of both signs, no identity at the link by the source (nir_search.c:282-285); refused. */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(a.zx + vec2(0.0, -0.0), 0.5, 1.0); }
