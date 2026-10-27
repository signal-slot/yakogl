precision mediump float;
uniform sampler2D l;
void main() { gl_FragColor = texture2D(l, vec2(0.5, 0.5)); }
