attribute highp vec4 a_position; void main() { float z = -0.0; gl_Position = (a_position.x < z) ? a_position * z : vec4(z, 0.5, 0.5, 1.0); }
