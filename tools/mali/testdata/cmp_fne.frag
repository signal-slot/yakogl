/* fne_hp of docs/notes/mali-compare-forms.md: float(v.x != v.y) at highp,
 * FCMP_OR.f32.ne.m1 of the raw varying lanes (the load is .v2, only x and y
 * are read). Drawn without --uniform-values (docs/notes/mali-compare-forms.md,
 * "Files, test, what it moves"). */
precision highp float;
precision highp int;
uniform highp int m;
varying vec4 v;
void main() {
    gl_FragColor = vec4(float(v.x != v.y), 0.0, 0.0, 1.0);
}
