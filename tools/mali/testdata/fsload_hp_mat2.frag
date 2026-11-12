/* A matrix of a highp shader: the lane read alone (one store). */
precision highp float;
varying mat2 m;
void main() { gl_FragColor = vec4(m[0].y, 0.5, 0.25, 1.0); }
