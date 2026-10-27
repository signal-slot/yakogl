/* Selector population: Whole position at attribute location 1, a vec3 varying of location 0 (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; Position::Whole(1)). */
attribute vec3 c; attribute vec4 p; varying vec3 v; void main(){ gl_Position = p; v = c; }
