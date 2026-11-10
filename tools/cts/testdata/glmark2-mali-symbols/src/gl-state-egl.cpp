/* Excerpt of glmark2 22c527cb src/gl-state-egl.cpp: the dlopen name lists
 * (the file has them near lines 335 and 651). Only the lists are kept. */
#include "gl-state-egl.h"

bool
EGLState::init_display_lib()
{
    const char *names[] = { "libEGL.so", "libEGL.so.1", 0 };
    return load_first(names);
}

bool
GLStateEGL::init_gl_extensions_lib()
{
    const char *names[] = { "libGLESv2.so", "libGLESv2.so.2", 0 };
    return load_first(names);
}
