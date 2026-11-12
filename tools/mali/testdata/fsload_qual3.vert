#version 300 es
/* A smooth varying and a flat one of its swizzle (the vertex shader of fsload_qual3). */
in vec4 a_pos; in vec4 a_a; in vec4 a_b;
out vec2 p; flat out vec2 q;
void main() {
  gl_Position = a_pos;
  p = a_a.xy; q = a_a.yx;
}
