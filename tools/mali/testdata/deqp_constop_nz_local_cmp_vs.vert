attribute highp vec4 a_position; void main() { const float z = -0.0; gl_Position = a_position.x < z ? a_position : vec4(0.5); }
