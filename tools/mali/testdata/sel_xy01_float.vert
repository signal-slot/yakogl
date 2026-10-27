/* Selector population: Xy01 position, a float varying of another attribute (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; a 1-component LD_ATTR and store). */
attribute vec2 pos; attribute float c; varying float v; void main(){ gl_Position = vec4(pos, 0.0, 1.0); v = c; }
