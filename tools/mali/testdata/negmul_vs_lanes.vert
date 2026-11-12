attribute highp vec4 a_position; attribute highp vec4 a_a2; 
void main() { gl_Position = a_position * -vec4(2.0, -3.0, 1.0, 0.5); }
