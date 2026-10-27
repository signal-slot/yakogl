/* S5 (if): an if with no else of nine min/max over uniforms (past the GL front end's limit of 8; load_ubo in bifrost's loop, which its peephole may not speculate): a BRANCHZ to the join and no jump (tools/mali/glsl2pan.py with s5_if.vert, pan_shc, Mesa 26.2.1; highp). */
precision highp float;
varying vec4 v;
uniform vec4 u;
void main() {
    float r = v.x * v.y;
    if (v.z > 0.0) {
        r = min(r, u.x);
        r = max(r, u.y);
        r = min(r, u.z);
        r = max(r, u.w);
        r = min(r, 0.5);
        r = max(r, -0.5);
        r = min(r, u.y);
        r = max(r, u.z);
        r = min(r, u.x);
    }
    gl_FragColor = vec4(r, 0.0, 0.0, 1.0);
}
