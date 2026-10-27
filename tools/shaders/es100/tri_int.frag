#version 100
/* v3d_triangle's int variant (TRI_INT): every int and bool operation the
 * V3D back end selects since 7b41538f changes the picture, and each
 * comparison has inside pixels on both of its sides.
 *   red:   i = int(r * 4.0) is 0..3 across the triangle (F2i); j = -i * 3 + 7
 *          (Ineg, Imul, Iadd, ConstI32); red = float(j + 5) / 12.0 (I2f):
 *          1, 0.75, 0.5, 0.25 for i = 0..3.
 *   green: float(i == 1) (Ieq, B2f) and float(i != 2) (Ine, B2f).
 *   blue:  a ternary on b, read by the very next add (the select result the
 *          scheduler fix pins), plus sign(g - 0.5) (Fsign). */
precision mediump float;
varying vec4 v_color;
void main() {
    int i = int(v_color.r * 4.0);
    int j = -i * 3 + 7;
    bool e = (i == 1);
    bool n = (i != 2);
    float t = v_color.b < 0.5 ? 0.25 : 0.75;
    float u = t + sign(v_color.g - 0.5) * 0.125;
    gl_FragColor = vec4(float(j + 5) / 12.0, float(e) * 0.5 + float(n) * 0.25 + 0.125, u, 1.0);
}
