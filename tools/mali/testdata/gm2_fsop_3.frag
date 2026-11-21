#version 100
/* reduction: mix of two float uniforms by the varying product, mediump */
precision mediump float;
uniform float c1;
uniform float c2;
varying vec2 uv;
void main() {
    float c = mix(c1, c2, uv.x * uv.y);
    gl_FragColor = vec4(c, c, c, 1.0);
}
