/* t_zw4_hp of docs/notes/mali-tex-coords.md: a highp texture coordinate
 * that is the .zw half of a vec4 varying, byte-identical to tex_xy4 and to
 * a whole vec2 varying (pan_nir_resize_varying_io trims the load to .v2 and
 * the vertex shader stores only the read pair). Drawn with --uniform-values. */
precision highp float;
uniform sampler2D s;
uniform vec2 u;
varying vec4 v;
void main() {
    gl_FragColor = texture2D(s, v.zw);
}
