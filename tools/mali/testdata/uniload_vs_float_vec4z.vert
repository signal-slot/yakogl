attribute highp vec4 a_position; uniform float f; uniform vec4 u;
void main() { gl_Position = a_position * u.z + f; }
