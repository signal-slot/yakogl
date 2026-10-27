/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * getopt.h for glmark2's options.cpp: Zephyr's getopt.h
 * (subsys/portability/posix/c_lib_ext/getopt) declares getopt_long but not
 * optarg/optind, which POSIX puts in <unistd.h> and glibc's getopt.h also
 * declares. options.cpp includes only <getopt.h>, as on glibc.
 */
#ifndef GLMARK2_GETOPT_H
#define GLMARK2_GETOPT_H

#include_next <getopt.h>
#include <unistd.h>

#endif /* GLMARK2_GETOPT_H */
