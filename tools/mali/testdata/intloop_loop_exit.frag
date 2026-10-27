#version 300 es
/* A loop whose bound is read at run time, different per fragment (the
 * unroller keeps it capped, with an exit flag; v3d back end: the counted
 * loop leaves when every lane's flag is set). The bound is 0..5, so the
 * early exit, not the cap of 4096 passes, is what ends it: the harness pins
 * the instructions run. */
precision highp float;
precision highp int;
in vec4 v_color;
out vec4 o_color;
void main() {
    int n = int(clamp(v_color.r, 0.0, 1.0) * 5.0);
    float s = 0.0;
    for (int i = 0; i < n; i++) {
        s += v_color.g + float(i);
    }
    o_color = vec4(s / 16.0, float(n) / 8.0, v_color.b, 1.0);
}
