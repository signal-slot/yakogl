/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* capi/eh_personality.c - the Rust EH personality for hosted builds.
 *
 * On hosted targets (Linux), the precompiled core and compiler_builtins are
 * built with unwind tables that name `rust_eh_personality` (from the rustc
 * 1.101 nightlies on: rustc's internal `__rustc::rust_eh_personality`,
 * v0-mangled with a hash per compiler), although YakoGL
 * never unwinds (panic = "abort"). Something must define it, and it must be a
 * real personality routine: a foreign unwind (a C++ exception thrown from an
 * OSAL or log callback, pthread_cancel) that crosses a Rust frame calls it.
 *
 * Forwarding to the GCC C personality is correct for these frames: Rust with
 * panic = "abort" emits only cleanup landing pads (the "cannot unwind" abort
 * pads of extern "C" functions), which is exactly what __gcc_personality_v0
 * handles for C's __attribute__((cleanup)). The unwind then reaches the abort
 * pad and the image stops with the panic message, instead of calling an empty
 * function as personality.
 *
 * Compiled only for hosted Rust targets (bare-metal targets reference no
 * personality). In libyakogl.a the name is yakogl_rust_eh_personality:
 * capi/assemble.cmake renames the Rust side's references, whichever name
 * they use, so this cannot collide with the personality of a Rust std
 * staticlib in the same program. A build that links the yakogl crate into its
 * own Rust staticlib (external linkage) and has no std, or appends this to an
 * archive it does not post-process, compiles this file with
 * -DYAKOGL_EH_PERSONALITY=<the name the Rust objects reference>, read off
 * them at build time (CMakeLists.txt, tools/hostharness/run.sh). */
#include <unwind.h>

#ifndef YAKOGL_EH_PERSONALITY
#define YAKOGL_EH_PERSONALITY yakogl_rust_eh_personality
#endif

_Unwind_Reason_Code __gcc_personality_v0(int version, _Unwind_Action actions,
                                         _Unwind_Exception_Class exception_class,
                                         struct _Unwind_Exception *exception_object,
                                         struct _Unwind_Context *context);

_Unwind_Reason_Code YAKOGL_EH_PERSONALITY(int version, _Unwind_Action actions,
                                         _Unwind_Exception_Class exception_class,
                                         struct _Unwind_Exception *exception_object,
                                         struct _Unwind_Context *context);

_Unwind_Reason_Code YAKOGL_EH_PERSONALITY(int version, _Unwind_Action actions,
                                         _Unwind_Exception_Class exception_class,
                                         struct _Unwind_Exception *exception_object,
                                         struct _Unwind_Context *context)
{
	return __gcc_personality_v0(version, actions, exception_class, exception_object, context);
}
