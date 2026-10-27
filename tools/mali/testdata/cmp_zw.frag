/* Compaction: a fragment shader that reads part of its varyings (zw). */
precision mediump float;
varying vec4 v_color;
void main() { gl_FragColor = vec4(v_color.zw, 0.25, 1.0); }
