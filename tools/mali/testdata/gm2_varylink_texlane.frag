precision mediump float;
varying vec3 vNormal;
varying vec2 vUv;
void main() { gl_FragColor = vec4(vNormal, 1.0) + vec4(vUv, 0.0, 0.0); }
