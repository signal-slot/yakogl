#version 100
/* A loop the unroller keeps (a break on data) that runs its whole count on
 * most lanes and breaks early on a few (v3d back end: the counted loop).
 * Every pass adds to a carried sum, so a pass too few or a carried value not
 * copied back changes the colour. */
precision mediump float;
varying vec4 v_color;
void main() {
    float s = 0.0;
    float t = v_color.b;
    for (int i = 0; i < 8; i++) {
        if (v_color.r > 0.9 && float(i) > v_color.g * 8.0)
            break;
        s += float(i) + v_color.g;
        t = t * 0.5 + 0.25;
    }
    gl_FragColor = vec4(s / 64.0, t, v_color.b, 1.0);
}
