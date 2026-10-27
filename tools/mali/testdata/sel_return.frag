/* Selector population: two constant pairs, then a return and code the return makes dead (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; the front end drops the code after it). */
precision mediump float; void main(){ gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); return; gl_FragColor = vec4(0.0); }
