#version 100
/* dEQP-GLES2.functional.shaders.conditionals.if.sequence_condition
 * (data/gles2/shaders/conditionals.test), its fragment case: a comma
 * sequence as the condition, and a comma sequence as the statement.
 * in0 is 3 * the interpolated red so that dEQP's inputs 0, 1 and 2 are
 * all in reach; out0 and out1 go to red and green. */
precision mediump float;
varying vec4 v_color;
void main() {
    float in0 = 3.0 * v_color.r;
    float out0;
    float out1;
    out0 = 0.0;
    out1 = 1.0;
    if (false, in0 >= 1.0)
        out0 = 1.0, out1 = 0.0;
    gl_FragColor = vec4(out0, out1, 0.0, 1.0);
}
