/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * What dEQP's framework references and a Zephyr image does not have: child
 * processes (tcu::CommandLine's subprocess execution modes, never selected
 * here) and three file-system calls. Each fails the way the real call fails
 * when the thing does not exist, so a path that reaches one reports an error
 * instead of misbehaving.
 */
#include <errno.h>
#include <stdbool.h>
#include <stddef.h>
#include <sys/types.h>
#include <sys/stat.h>
#include "deProcess.h"

deProcess *deProcess_create(void)
{
	return NULL;
}

void deProcess_destroy(deProcess *process)
{
	(void)process;
}

bool deProcess_start(deProcess *process, const char *commandLine, const char *workingDirectory)
{
	(void)process;
	(void)commandLine;
	(void)workingDirectory;
	return false;
}

bool deProcess_isRunning(deProcess *process)
{
	(void)process;
	return false;
}

int deProcess_getExitCode(const deProcess *process)
{
	(void)process;
	return -1;
}

char *getcwd(char *buf, size_t size)
{
	(void)buf;
	(void)size;
	errno = ENOSYS;
	return NULL;
}

int lstat(const char *path, struct stat *st)
{
	(void)path;
	(void)st;
	errno = ENOENT;
	return -1;
}

ssize_t readlink(const char *path, char *buf, size_t size)
{
	(void)path;
	(void)buf;
	(void)size;
	errno = ENOENT;
	return -1;
}
