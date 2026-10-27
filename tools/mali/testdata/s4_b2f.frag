/* S4 ALU row: flt then b2f then fmul (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; highp). */
precision highp float; varying vec4 v; void main(){ float r = float(v.x > 0.5) * v.y; gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }
