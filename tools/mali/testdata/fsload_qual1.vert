#version 300 es
/* A smooth and a flat varying of equal value (the vertex shader of fsload_qual1). */
in vec4 a_pos; in vec4 a_a; in vec4 a_b;
out vec2 p; flat out vec2 q;
void main() {
  gl_Position = a_pos;
  p = a_a.xy; q = a_a.xy;
}
