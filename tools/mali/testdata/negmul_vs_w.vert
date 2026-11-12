attribute highp vec4 a_position; attribute highp vec4 a_b; void main() { gl_Position = vec4(a_position.xyz, a_b.w * -2.0); }
