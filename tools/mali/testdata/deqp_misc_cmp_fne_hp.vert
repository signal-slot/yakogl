/* deqp_misc_cmp_fne_hp: mali-compare-forms.md's tc.vert (a position and one vec4 varying). */
attribute vec4 a_pos;
attribute vec4 a_v;
varying vec4 v;
void main() { gl_Position = a_pos; v = a_v; }
