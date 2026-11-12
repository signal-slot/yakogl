attribute highp vec4 a_position; uniform vec4 u;
void main() { gl_Position = a_position * u.z; }
