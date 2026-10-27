/* Selector population: pairs (Vv, Vc), alpha constant 1.0, a vec3 varying (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; Pair::Vc whose constant is in the table). */
precision mediump float; varying vec3 v; void main(){ gl_FragColor = vec4(v, 1.0); }
