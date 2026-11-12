/* t.xz: a.x beside the constant 0.5 in the lanes the colour reads: a whole (.v4). */
precision mediump float; varying vec4 a; void main() { vec4 t = vec4(a.xy, 0.5, 1.0); gl_FragColor = vec4(1.0); gl_FragColor.xz = t.xz; }
