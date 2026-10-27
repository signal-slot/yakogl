precision mediump float;
uniform bool u_b;
uniform bvec3 u_bv;
uniform vec4 u_v;
void main() {
    bvec4 gt = greaterThan(u_v, vec4(0.5));
    float r = (u_b ? 0.5 : 0.0) + (any(u_bv) ? 0.25 : 0.0) + (all(u_bv) ? 0.25 : 0.0);
    float g = any(gt) ? 1.0 : 0.0;
    float b = all(gt) ? 1.0 : 0.0;
    gl_FragColor = vec4(r, g, b, 1.0);
}
