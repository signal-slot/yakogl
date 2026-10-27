/* Compaction: a fragment shader that reads part of its varyings (xyz). */
precision mediump float;
varying vec4 v_color;
void main() { gl_FragColor = vec4(v_color.xyz, 1.0); }
