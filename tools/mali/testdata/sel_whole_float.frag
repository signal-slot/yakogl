/* Selector population: pairs (Vc, Cc), the Vc constant promoted, the Cc word a table entry (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; Pair::Vc with 1.0). */
precision mediump float; varying float v; void main(){ gl_FragColor = vec4(v, 1.0, 0.0, 1.0); }
