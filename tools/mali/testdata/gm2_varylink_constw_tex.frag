precision mediump float;
uniform sampler2D tex;
varying vec4 Color;
varying vec2 TextureCoord;
void main() { gl_FragColor = texture2D(tex, TextureCoord) * Color; }
