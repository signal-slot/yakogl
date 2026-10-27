/* Selector population: a pair of a varying lane and 0.0, which Mesa selects as U16_TO_U32 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; refused as MaliError::ColourZeroHalf). */
precision mediump float; varying float v; void main(){ gl_FragColor = vec4(v, 0.0, 0.0, 1.0); }
