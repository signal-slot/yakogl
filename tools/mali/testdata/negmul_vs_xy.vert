attribute highp vec4 a_position; void main() { gl_Position = vec4(a_position.xy * -3.0, a_position.zw); }
