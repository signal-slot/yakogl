/* S5 (if): an if of one operation over varyings loaded before it, which nir_opt_peephole_select makes a bcsel (CSEL) (tools/mali/glsl2pan.py with s5_if.vert, pan_shc, Mesa 26.2.1; highp). */
precision highp float;
varying vec4 v;
uniform float u;
void main() {
    float r;
    if (u > 0.0) {
        r = v.x * v.y + v.z;
    } else {
        r = v.w;
    }
    gl_FragColor = vec4(r, 0.0, 0.0, 1.0);
}
