#version 300 es
/* One vec4 varying (the vertex shader of fsload_cancel8). */
in vec4 a_pos; in vec4 a_a; in vec4 a_b;
out vec4 a;
void main() {
  gl_Position = a_pos;
  a = a_a;
}
