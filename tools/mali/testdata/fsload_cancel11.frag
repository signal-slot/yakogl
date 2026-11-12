/* A local of a.xy built twice into the colour: the lanes read (.v2). */
precision mediump float; varying vec4 a; void main() { mediump vec2 t = a.xy; gl_FragColor = vec4(t, t); }
