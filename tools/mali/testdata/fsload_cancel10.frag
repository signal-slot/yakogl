/* A constructor of the varying's lanes alone: one vector of its f2fmp, undone at the link; the lanes read (.v2). */
precision mediump float; varying vec4 a; void main() { gl_FragColor = vec4(a.x, a.y, a.x, a.y); }
