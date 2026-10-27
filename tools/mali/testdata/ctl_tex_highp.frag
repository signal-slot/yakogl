/* Control: highp FS with a sampler2D, a uniform and gl_FragCoord. */
precision highp float;
uniform sampler2D s;
uniform vec4 u_tint;
varying vec2 v_uv;
void main() { gl_FragColor = texture2D(s, v_uv) * u_tint + vec4(gl_FragCoord.y * 0.001); }
