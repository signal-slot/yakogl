/* The constructor stored whole: its constants beside a.xy keep a whole (.v4). */
precision mediump float; varying vec4 a; void main() { vec4 t = vec4(a.xy, 0.5, 1.0); gl_FragColor = t; }
