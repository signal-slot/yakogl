attribute highp vec4 a_position; void main() { gl_Position = vec4(a_position.xyz, -a_position.w); }
