/* deqp_misc_cmp_fne_hp: a highp float != (fneu) of two varying lanes; docs/notes/mali-compare-forms.md's fne_hp, drm-shim reference with --uniform-values. */
precision highp float;
precision highp int;
uniform highp int m;
varying vec4 v;
void main() { gl_FragColor = vec4(float(v.x != v.y), 0.0, 0.0, 1.0); }
