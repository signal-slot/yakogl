/* Selector population: pairs (Vv, Cc), alpha constant, a vec2 varying (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; Pair::Vv then Pair::Cc). */
precision mediump float; varying vec2 v; void main(){ gl_FragColor = vec4(v, 0.0, 1.0); }
