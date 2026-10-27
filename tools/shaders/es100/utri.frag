#version 100
/* The interpolated-colour FS with a uniform scale and bias per channel: the
 * fragment half of v3d_triangle's uniform variant (TRI_UNIFORMS). Two vec4
 * uniforms, so a wrong word index reads a value from the other one. */
precision mediump float;
uniform vec4 u_scale;
uniform vec4 u_bias;
varying vec4 v_color;
void main() { gl_FragColor = v_color * u_scale + u_bias; }
