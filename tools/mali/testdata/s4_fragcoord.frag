/* S4 gl_FragCoord: the preloaded pixel position, its centre and the y transform, copied to a highp variable first as femtovg's ditherNoise(gl_FragCoord.xy) is (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; highp). */
precision highp float; varying vec4 v; void main(){ vec2 q = gl_FragCoord.xy; gl_FragColor = vec4(q * 0.01, v.x, 1.0); }
