/* Compaction: a fragment shader that reads part of its varyings (axby). */
precision mediump float;
varying vec4 v_a; varying vec4 v_b;
void main() { gl_FragColor = vec4(v_a.x, v_b.y, 0.25, 1.0); }
