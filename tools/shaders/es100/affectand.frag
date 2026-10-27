/* dEQP-GLES2 operator.sequence.side_effects.affect_and, verbatim: the
 * compare `u_false == sideEffectA` is both stored to the `&&`'s temporary
 * and the `if`'s condition. Selection counted only the store as its use,
 * wrote the compare into the slot's register, and the if-converted arm read
 * the value's own register, which nothing wrote: funcB's store took effect
 * on the i.MX8MP whatever u_false said (2026-09-24). Green for u_false =
 * false, red for true. */
precision mediump float;
uniform bool u_false;
bool sideEffectA = false;
bool funcA() {
    sideEffectA = true;
    return true;
}
bool sideEffectB = false;
bool funcB() {
    sideEffectB = true;
    return true;
}
void main() {
    bool b = (funcA(), u_false == sideEffectA && funcB());
    gl_FragColor = (!b && sideEffectA && !sideEffectB) ? vec4(0, 1, 0, 1) : vec4(1, 0, 0, 1);
}
