#version 100
/* Two samplers: the unit-0 texture weighted 0.5 plus the unit-1 texture
 * weighted 0.25. ES 1.00 twin of gl_test_texture2_fs_src. */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex0;
uniform sampler2D u_tex1;
void main() {
    gl_FragColor = vec4(texture2D(u_tex0, v_uv).rgb * 0.5 + texture2D(u_tex1, v_uv).rgb * 0.25, 1.0);
}
