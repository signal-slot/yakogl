precision mediump float;
varying vec2 TextureCoord;
varying vec3 NormalEye;
varying vec3 TangentEye;
varying vec3 BitangentEye;
void main() { gl_FragColor = vec4(NormalEye - TangentEye - BitangentEye, 1.0) + vec4(TextureCoord, 0.0, 0.0); }
