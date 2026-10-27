/* Control: the same FS at mediump (GLSL ES 1.00 lowers mediump arithmetic to fp16 in Mesa's GLSL path). */
precision mediump float;
uniform sampler2D s;
uniform vec4 u_tint;
varying vec2 v_uv;
void main() { gl_FragColor = texture2D(s, v_uv) * u_tint + vec4(gl_FragCoord.y * 0.001); }
