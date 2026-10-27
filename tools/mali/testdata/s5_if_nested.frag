/* S5 (if): an if nested in the then side of another, every side nine min/max over uniforms (past the limits): four real blocks a level, the inner join inside the outer then side (tools/mali/glsl2pan.py with s5_if.vert, pan_shc, Mesa 26.2.1; highp). */
precision highp float;
varying vec4 v;
uniform vec4 u;
void main() {
    float r = v.x;
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
        if (v.w > 0.0) {
            r = max(r, u.x);
            r = min(r, u.y);
            r = max(r, u.z);
            r = min(r, u.w);
            r = max(r, 0.25);
            r = min(r, 0.75);
            r = max(r, u.y);
            r = min(r, u.z);
            r = max(r, u.x);
        } else {
            r = min(r, u.w);
            r = max(r, u.z);
            r = min(r, u.y);
            r = max(r, u.x);
            r = min(r, 0.125);
            r = max(r, 0.0625);
            r = min(r, u.z);
            r = max(r, u.y);
            r = min(r, u.w);
        }
    } else {
        r = max(r, u.w);
        r = min(r, u.z);
        r = max(r, u.y);
        r = min(r, u.x);
        r = max(r, 0.5);
        r = min(r, 0.75);
        r = max(r, u.z);
        r = min(r, u.y);
        r = max(r, u.w);
    }
    gl_FragColor = vec4(r, 0.0, 0.0, 1.0);
}
