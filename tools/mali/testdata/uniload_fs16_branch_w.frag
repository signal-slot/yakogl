precision mediump float;
uniform sampler2D ta;
uniform sampler2D tb;
uniform vec4 u;
void main() {
    vec3 c = vec3(0.0);
    if (u.w > 0.5) {
        c.r = texture2D(ta, vec2(0.5)).r;
    } else {
        c.r = texture2D(tb, vec2(0.5)).r;
    }
    gl_FragColor = vec4(c, 1.0);
}
