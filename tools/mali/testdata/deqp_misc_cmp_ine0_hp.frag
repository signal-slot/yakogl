/* deqp_misc_cmp_ine0_hp: a highp int uniform != 0 (ine); docs/notes/mali-compare-forms.md's ine0_hp, drm-shim reference with --uniform-values. */
precision highp float;
precision highp int;
uniform highp int m;
varying vec4 v;
void main() { gl_FragColor = vec4(float(m != 0), 0.0, 0.0, 1.0); }
