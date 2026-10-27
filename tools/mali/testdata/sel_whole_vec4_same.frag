/* Selector population: pairs (Vv, Vv), alpha from the varying, then a trailing return (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; Pair::Vv twice). */
precision mediump float; varying vec4 v; void main(){ gl_FragColor = v; return; }
