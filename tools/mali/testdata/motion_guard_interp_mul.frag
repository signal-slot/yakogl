/* Motion guard pair: a highp product of an interpolated lane by a constant, stored straight into the colour (Mesa 26.2.1 moves it). */
precision highp float; varying vec4 v; void main(){ float r = v.x * 2.0; gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }
