/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Stand-in for libdrm's xf86drm.h, which Mesa 26.2.1 src/broadcom/common/v3d_util.h
 * includes for the one inline v3d_ioctl() that calls drmIoctl(). Nothing in
 * tools/v3d calls v3d_ioctl(); the stub keeps the libdrm development package
 * out of the gate's needs.
 */
#ifndef XF86DRM_STUB_H
#define XF86DRM_STUB_H
int drmIoctl(int fd, unsigned long request, void *arg);
#endif
