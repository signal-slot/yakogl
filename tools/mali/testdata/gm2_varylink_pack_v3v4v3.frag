precision mediump float;
varying vec3 vertex_normal;
varying vec4 vertex_position;
varying vec3 eye_direction;
void main() { gl_FragColor = vec4(vertex_normal + eye_direction + vertex_position.xyz, vertex_position.w); }
