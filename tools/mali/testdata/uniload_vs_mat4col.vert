attribute highp vec4 a_position; uniform mat4 m;
void main() { gl_Position = m[2] * a_position.x + m[0].x; }
