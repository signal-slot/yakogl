/* Selector population: pairs (Vc, Cc), alpha constant, a float varying (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; Pair::Vc with a promoted constant, Pair::Cc). */
precision mediump float; varying float v; void main(){ gl_FragColor = vec4(v, 0.5, 0.25, 1.0); }
