// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
let pr = r.cfg.partial_render != 0;
wr32(a, UNRELATED); // PM_THING in a comment
if pr {
    wr32(a, PM_THING);
}
#[cfg(test)]
mod tests {
    fn t() {
        if pr { x(UNRELATED); }
    }
}
