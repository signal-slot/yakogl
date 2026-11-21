#version 100
/* reduction: mix of two vec3 uniforms by the varying product, highp, alpha from a uniform */
precision highp float;
uniform vec3 color1;
uniform vec3 color2;
uniform float a;
varying vec2 uv;
void main() {
    gl_FragColor = vec4(mix(color1, color2, uv.x * uv.y), a);
}
