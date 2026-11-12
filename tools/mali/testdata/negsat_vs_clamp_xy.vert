attribute highp vec4 a_position; attribute highp vec4 a_a2; 
void main() { gl_Position = vec4(clamp(a_position.xy * -2.0, 0.0, 1.0), a_position.zw); }
