/* Selector population: Whole position, a float varying of another attribute (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; VsVarying comps 1). */
attribute vec4 p; attribute float c; varying float v; void main(){ gl_Position = p; v = c; }
