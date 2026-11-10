/* t_swz_hp of docs/notes/mali-tex-coords.md: a highp texture coordinate
 * that is the swapped vec2 varying v.yx (the swapped pair is two moves into
 * the staging). Drawn with --uniform-values. */
precision highp float;
uniform sampler2D s;
uniform vec2 u;
varying vec2 v;
void main() {
    gl_FragColor = texture2D(s, v.yx);
}
