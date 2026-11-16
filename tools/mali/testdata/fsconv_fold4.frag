/* An identity on some lanes only (z, w of a * vec4(2.0, 2.0, 1.0, 1.0)): the operation stays an fp16 one at the link, z and w are converted with x and y. */
precision mediump float; varying vec4 a; void main() { gl_FragColor = a * vec4(2.0, 2.0, 1.0, 1.0); }
