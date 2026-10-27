#version 100
attribute vec4 a_pos;
uniform vec3 u_major;
uniform vec3 u_sx;
uniform vec3 u_ty;
varying vec3 v_dir;
void main() {
    v_dir = u_major + a_pos.x * u_sx + a_pos.y * u_ty;
    gl_Position = a_pos;
}
