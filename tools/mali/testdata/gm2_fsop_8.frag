#version 100
/* reduction: the varying product alone, no uniform, mediump */
precision mediump float;

varying vec2 uv;
void main() {
    float t = uv.x * uv.y;
    gl_FragColor = vec4(t, t, t, 1.0);
}
