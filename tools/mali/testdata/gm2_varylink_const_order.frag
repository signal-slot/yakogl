precision mediump float;
varying vec4 Color;
varying vec4 ShadowCoord;
void main() { gl_FragColor = vec4(Color.rgb * ShadowCoord.xyz * ShadowCoord.w, 1.0); }
