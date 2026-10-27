/* S5 (if): an if kept as blocks (nine min/max over uniforms in the then side) whose join has a phi of two constants, which bi_out_of_ssa moves with IADD_IMM (tools/mali/glsl2pan.py with s5_if.vert, pan_shc, Mesa 26.2.1; highp). */
precision highp float;
varying vec4 v;
uniform vec4 u;
void main() {
    float r, s;
    if (v.z > 0.0) {
        r = min(v.x, u.x);
        r = max(r, u.y);
        r = min(r, u.z);
        r = max(r, u.w);
        r = min(r, 0.5);
        r = max(r, -0.5);
        r = min(r, u.y);
        r = max(r, u.z);
        r = min(r, u.x);
        s = 0.25;
    } else {
        r = v.y;
        s = 0.75;
    }
    gl_FragColor = vec4(r, s, 0.0, 1.0);
}
