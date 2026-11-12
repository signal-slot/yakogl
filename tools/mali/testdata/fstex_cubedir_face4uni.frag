precision mediump float; uniform samplerCube s; uniform float u; varying vec2 a2;
void main() {
    mediump float x = a2.x*2.0-1.0; mediump float y = a2.y*2.0-1.0; gl_FragColor = textureCube(s, vec3(-x, -y, u));
}
