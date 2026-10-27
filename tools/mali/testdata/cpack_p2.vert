/* Two vec2 varyings in one slot, the second from a part of the position (cpack_p2). */
attribute vec4 a_pos; attribute vec2 a_t; varying vec2 v_a; varying vec2 v_b;
void main() { gl_Position = a_pos; v_a = a_t; v_b = a_pos.zw; }
