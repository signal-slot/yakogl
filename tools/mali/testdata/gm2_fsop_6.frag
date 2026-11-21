#version 100
/* reduction: a vec3 uniform first in a sum with a varying product, highp */
precision highp float;
uniform vec3 color1;
varying vec2 uv;
void main() {
    gl_FragColor = vec4(color1 + vec3(uv.x * uv.y), 1.0);
}
