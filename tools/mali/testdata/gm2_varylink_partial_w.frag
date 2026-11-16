precision mediump float;
varying vec3 vertex_normal;
varying vec4 vertex_position;
void main() { gl_FragColor = vec4(vertex_normal * vertex_position.xyz, 1.0); }
