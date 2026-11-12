attribute highp vec4 a_position; void main() { gl_Position = -a_position.x < 0.5 ? a_position : vec4(0.5); }
