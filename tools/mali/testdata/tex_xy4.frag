/* t_xy4_hp of docs/notes/mali-tex-coords.md: a highp texture coordinate
 * that is the .xy half of a vec4 varying (the Mesa 26.2.1 reference's load
 * is trimmed to .v2; the tip refuses it as TexCoord at nir_fs.rs). Drawn
 * with --uniform-values, like the note's variants. */
precision highp float;
uniform sampler2D s;
uniform vec2 u;
varying vec4 v;
void main() {
    gl_FragColor = texture2D(s, v.xy);
}
