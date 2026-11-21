precision mediump float;
uniform bool u_false;
bool sideEffectA = false;
bool funcA() {
    sideEffectA = true;
    return false;
}
bool sideEffectB = false;
bool funcB() {
    sideEffectB = true;
    return false;
}
void main() {
    bool b = (funcA(), (u_false == !sideEffectA) || funcB());
    gl_FragColor = (b && sideEffectA && !sideEffectB) ? vec4(0, 1, 0, 1) : vec4(1, 0, 0, 1);
}
