/* gl_test case 171 (viewwin): the raw-memory check follows the window's byte
 * order. The case needs a GPU that renders, which no host build has, so this
 * checks the one decision it makes: the expected word of a pixel for a
 * backend whose RGBA8888 target is stored R,G,B,A (PowerVR, V3D) and for one
 * that stores B,G,R,A (the GC7000, rt_rgba8888_is_bgra).
 *
 * It includes gl_test.c itself (GL_TEST_HOST), so it runs the real
 * viewwin_word(); the GL/EGL/OSAL entry points are never called, so the link
 * ignores them. Run from the repository root:
 *   cc -std=gnu11 -DGL_TEST_HOST=1 -Iinclude -Isrc -Isrc/osal -Idriver/tools/pvrlab \
 *     -Idriver/src tests/gltest_viewwin_order.c -o /tmp/vw \
 *     -Wl,--unresolved-symbols=ignore-all && /tmp/vw
 */
#include "../driver/tools/pvrlab/gl_test.c"

static struct yakogl_backend fake;
const struct yakogl_backend *g_backend = &fake;

static int check(const char *what, uint32_t got, uint32_t want)
{
	printf("%-44s got 0x%08x want 0x%08x %s\n", what, got, want, got == want ? "ok" : "FAIL");
	return got != want;
}

int main(void)
{
	int bad = 0;

	fake.rt_rgba8888_is_bgra = false;
	bad |= check("R,G,B,A back end: red", viewwin_word(0xff0000ff), 0xff0000ff);
	fake.rt_rgba8888_is_bgra = true;
	bad |= check("B,G,R,A back end (GC7000): red", viewwin_word(0xff0000ff), 0xffff0000);
	bad |= check("B,G,R,A back end: blue", viewwin_word(0xffff0000), 0xff0000ff);
	bad |= check("B,G,R,A back end: only R and B swap", viewwin_word(0xff8040c0), 0xffc04080);
	return bad;
}
