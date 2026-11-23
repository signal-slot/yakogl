/* a cut-down gl_test.c: only the case table the splitter reads */
static const char *const case_names[] = {
	[0] = "triangle", [1] = "uniform", [2] = "texture", [3] = "depth", [4] = "cull",
	[5] = "scissor", [7] = "all", [8] = "blend",
	/* a comment with [99] = "not a case" in it */
	[10] = "prims", [11] = "persist", [25] = "stress",
};
