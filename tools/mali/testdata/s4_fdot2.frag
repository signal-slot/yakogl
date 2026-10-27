/* S4 ALU row: fdot2 (two fmuls and an fadd on Valhall) (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; highp). */
precision highp float; varying vec4 v; void main(){ float r = dot(v.xy, v.zw); gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }
