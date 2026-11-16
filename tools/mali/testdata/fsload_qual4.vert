#version 300 es
/* Two flat varyings of equal value (the vertex shader of fsload_qual4). */
in vec4 a_pos; in vec4 a_a; in vec4 a_b;
flat out vec2 p; flat out vec2 q;
void main() {
  gl_Position = a_pos;
  p = a_a.xy; q = a_a.xy;
}
