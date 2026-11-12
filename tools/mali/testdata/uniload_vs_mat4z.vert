attribute highp vec4 a_position; uniform mat4 m;
void main() { gl_Position = a_position * m[1].z; }
