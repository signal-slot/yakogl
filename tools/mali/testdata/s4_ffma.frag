/* S4 ALU row: ffma (fma, or fmul+fadd if Mesa does not fuse) (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; highp). */
precision highp float; varying vec4 v; void main(){ float r = v.x * v.y + v.z; gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }
