#version 100
/* glmark2 22c527cb ideas, vertex shaders bf4bdbb0 (the table) and c265c68a
 * (the paper): a one-armed `if` on two compares of a float uniform with
 * constants joined by &&, whose arm scales a value computed from the
 * attribute (a lit term) by an expression of the uniform. */
attribute vec3 a_pos;
uniform mat4 u_proj;
uniform mat4 u_mv;
uniform vec3 u_lpos;
uniform vec3 u_ldir;
uniform float u_time;
varying vec4 v_col;
void main() {
    gl_Position = u_proj * u_mv * vec4(a_pos, 1.0);
    vec3 l = normalize(u_lpos - a_pos);
    float c = max(dot(l, u_ldir), 0.0);
    c = c * c * l.y;
    if ((u_time > 10.0) && (u_time < 12.0)) {
        c *= 1.0 - (u_time - 10.0) * 0.5;
    }
    v_col = vec4(c, c, c, 1.0);
}
