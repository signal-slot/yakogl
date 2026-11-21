#version 100
/* reduction: mix of two vec3 uniforms by one varying lane, mediump */
precision mediump float;
uniform vec3 color1;
uniform vec3 color2;
varying vec2 uv;
void main() {
    gl_FragColor = vec4(mix(color1, color2, uv.x), 1.0);
}
