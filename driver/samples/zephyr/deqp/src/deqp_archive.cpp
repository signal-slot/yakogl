/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * A tcu::Archive over the test data linked into the image: there is no file
 * system on the board, and the data (1.1 MB for GLES2) fits in RAM easily.
 */
#include "deqp_archive.hpp"
#include "deqp_data.h"
#include "tcuResource.hpp"
#include "tcuDefs.hpp"
#include <cstring>

namespace
{

class MemResource : public tcu::Resource
{
public:
    MemResource(const char *name, const uint8_t *bytes, uint32_t size)
        : tcu::Resource(name), m_bytes(bytes), m_size(size), m_pos(0)
    {
    }
    void read(uint8_t *dst, int numBytes)
    {
        if (numBytes < 0 || (uint32_t)numBytes > m_size - m_pos)
            throw tcu::ResourceError("read past the end of an embedded resource", getName().c_str(), __FILE__, __LINE__);
        memcpy(dst, m_bytes + m_pos, (size_t)numBytes);
        m_pos += (uint32_t)numBytes;
    }
    uint32_t getSize(void) const
    {
        return m_size;
    }
    uint32_t getPosition(void) const
    {
        return m_pos;
    }
    void setPosition(uint32_t position)
    {
        m_pos = position > m_size ? m_size : position;
    }

private:
    const uint8_t *m_bytes;
    uint32_t m_size;
    uint32_t m_pos;
};

} // namespace

tcu::Resource *MemArchive::getResource(const char *name) const
{
    size_t lo = 0, hi = deqp_num_files;

    while (lo < hi)
    {
        const size_t mid = (lo + hi) / 2;
        const int c      = strcmp(deqp_files[mid].name, name);

        if (c == 0)
            return new MemResource(name, deqp_files[mid].bytes, deqp_files[mid].size);
        if (c < 0)
            lo = mid + 1;
        else
            hi = mid;
    }
    throw tcu::ResourceError("no embedded resource", name, __FILE__, __LINE__);
}
