/* S4 ALU row: f2i32, ilt, b2f (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; highp). */
precision highp float; varying vec4 v; void main(){ float r = float(int(v.x) < int(v.y)); gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }
