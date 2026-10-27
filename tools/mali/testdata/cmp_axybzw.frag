/* Compaction: a fragment shader that reads part of its varyings (axybzw). */
precision mediump float;
varying vec4 v_a; varying vec4 v_b;
void main() { gl_FragColor = vec4(v_a.xy, v_b.zw); }
