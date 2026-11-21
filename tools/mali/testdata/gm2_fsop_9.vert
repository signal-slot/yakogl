#version 100
/* reduction: mix of two vec3 uniforms by the varying product, highp, alpha from a uniform */
attribute vec2 position;
attribute vec2 uvIn;
varying vec2 uv;
void main() {
    uv = uvIn;
    gl_Position = vec4(position, 1.0, 1.0);
}
