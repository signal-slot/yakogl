#version 100
/* reduction: a uniform times a varying product, mediump */
precision mediump float;
uniform vec3 color1;
varying vec2 uv;
void main() {
    gl_FragColor = vec4(color1 * (uv.x * uv.y), 1.0);
}
