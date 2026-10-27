/* Selector population: pairs (Vv, Cc) reading the second of two declared varyings (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; plan_fs skips a varying it does not read). */
precision mediump float; varying vec4 u; varying vec2 v; void main(){ gl_FragColor = vec4(v, 0.0, 1.0); }
