/* Reads tsf_vs.vert's varying whole. */
precision mediump float; varying vec2 v_uv; void main() { gl_FragColor = vec4(v_uv, 0.25, 1.0); }
