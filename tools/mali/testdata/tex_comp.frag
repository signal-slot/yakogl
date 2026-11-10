/* t_comp_hp of docs/notes/mali-tex-coords.md: a highp computed texture
 * coordinate v * u (the product is computed in place, so the value pair is
 * already the staging). Drawn with --uniform-values. */
precision highp float;
uniform sampler2D s;
uniform vec2 u;
varying vec2 v;
void main() {
    gl_FragColor = texture2D(s, v * u);
}
