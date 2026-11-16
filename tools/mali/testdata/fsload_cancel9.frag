/* A dead tree of a.z beside gl_FragColor.xy = a.xy: the lanes read. */
precision mediump float;
varying vec4 a;
void main() { float k = a.z * 2.0; gl_FragColor = vec4(1.0); gl_FragColor.xy = a.xy; }
