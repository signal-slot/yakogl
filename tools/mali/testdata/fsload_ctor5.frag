/* t.xy of vec4(a.x, b.y, ..): two varyings in the lanes read, both whole. */
precision mediump float; varying vec4 a; varying vec4 b; void main() { vec4 t = vec4(a.x, b.y, 0.5, 1.0); gl_FragColor = vec4(1.0); gl_FragColor.xy = t.xy; }
