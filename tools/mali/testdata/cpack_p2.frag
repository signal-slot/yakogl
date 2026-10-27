/* Reads cpack_p2.vert's two varyings whole. */
precision mediump float; varying vec2 v_a; varying vec2 v_b;
void main() { gl_FragColor = vec4(v_a, v_b); }
