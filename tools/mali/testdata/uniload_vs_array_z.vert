attribute highp vec4 a_position; uniform vec4 u[2];
void main() { gl_Position = a_position * u[1].z; }
