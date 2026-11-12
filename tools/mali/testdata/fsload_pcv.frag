/* gl_PointCoord is loaded as its two lanes (LD_VAR_SPECIAL.v2) where
 * the colour reads x alone, beside two lanes of v_color (.v4). */
precision mediump float;
varying vec4 v_color;
void main() { gl_FragColor = vec4(v_color.x, v_color.z, gl_PointCoord.x, 1.0); }
