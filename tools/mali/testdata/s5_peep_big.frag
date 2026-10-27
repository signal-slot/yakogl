/* S5 (if): an if of twelve operations (under the limit of 64) over varyings loaded before it: a bcsel (CSEL) too (tools/mali/glsl2pan.py with s5_if.vert, pan_shc, Mesa 26.2.1; highp). */
precision highp float;
varying vec4 v;
uniform float u;
void main() {
    float r;
    if (u > 0.0) {
        float a = v.x * v.y + v.z;
        float b = a * v.w - v.x;
        float c = min(a, b) + max(b, 0.5);
        float d = c * c + a * 0.25;
        float e = sqrt(d) + 1.0 / c;
        r = clamp(e * 0.5, 0.0, 1.0) * v.y;
    } else {
        r = v.w;
    }
    gl_FragColor = vec4(r, 0.0, 0.0, 1.0);
}
