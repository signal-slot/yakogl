#version 100
/* #1385 (b), plan #153 uniarray: a vec4 uniform array read at constant
 * indices in the vertex stage, the shape of gl_test cases 149 vsunits
 * (`u_w[k / 4].x` weights) and 150 shprobe (`u_p[n].x * u_p[m]`) without
 * their samplers, the varying computed from the attribute so the link
 * keeps it (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; the pair's
 * fragment shader is uniarray_vs.frag). */
attribute vec4 a_pos;
uniform vec4 u_w[4];
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = u_w[0].x * a_pos + u_w[3].y * u_w[2];
}
