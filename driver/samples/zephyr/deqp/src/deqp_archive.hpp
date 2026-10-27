/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#ifndef DEQP_ARCHIVE_HPP
#define DEQP_ARCHIVE_HPP
#include "tcuResource.hpp"
class MemArchive : public tcu::Archive
{
public:
    tcu::Resource *getResource(const char *name) const;
};
#endif
