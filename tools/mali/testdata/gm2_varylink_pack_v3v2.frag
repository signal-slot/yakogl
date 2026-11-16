precision mediump float;
varying vec3 vTangent;
varying vec3 vBinormal;
varying vec3 vNormal;
varying vec2 vUv;
varying vec3 vViewPosition;
void main() { gl_FragColor = vec4(vTangent + vBinormal + vNormal + vViewPosition, 1.0) + vec4(vUv, 0.0, 0.0); }
