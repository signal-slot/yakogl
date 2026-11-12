attribute highp vec4 a_position; attribute highp vec4 a_a2; 
void main() { gl_Position = clamp(a_position * -2.0, 0.0, 1.0); }
