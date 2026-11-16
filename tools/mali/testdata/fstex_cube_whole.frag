#version 100
precision mediump float;
uniform samplerCube u_s;
varying vec3 v_dir;
void main() {
    gl_FragColor = textureCube(u_s, v_dir);
}
