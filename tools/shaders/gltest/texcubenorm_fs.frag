#version 100
precision mediump float;
uniform samplerCube u_s;
varying vec3 v_dir;
void main() {
    vec3 a = abs(v_dir);
    gl_FragColor = textureCube(u_s, v_dir / max(max(a.x, a.y), a.z));
}
