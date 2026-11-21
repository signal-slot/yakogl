#version 100
/* reduction: a vec3 uniform first in a sum with a varying product, mediump */
attribute vec2 position;
attribute vec2 uvIn;
varying vec2 uv;
void main() {
    uv = uvIn;
    gl_Position = vec4(position, 1.0, 1.0);
}
