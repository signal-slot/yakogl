attribute highp vec4 a_position; uniform vec2 a; uniform vec4 u;
void main() { gl_Position = a_position * u.w + a.x; }
