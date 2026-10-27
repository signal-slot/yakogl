/* S4 ALU row: ffract (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; highp). */
precision highp float; varying vec4 v; void main(){ float r = fract(v.x); gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }
