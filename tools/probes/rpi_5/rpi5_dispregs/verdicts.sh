#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_dispregs/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_dispregs image (src/main.c) and prints the
# answers to docs/notes/rpi5-display.md section 4, Q3, Q5 and Q6, in this
# order, always all of them (a missing line is said, not skipped):
#
#   rpi5_dispregs verdict: q3 pv<n>: <running|stopped|not read> ... (one line per valve)
#   rpi5_dispregs verdict: q3: <answer>
#       which pixel valve runs (the repo's mapping is pv0 -> HDMI0, pv1 -> HDMI1,
#       driver/docs/rpi5-display.md:107) and whether the valve's active width
#       is half the HVS channel's FWIDTH: `two pixels per clock: yes` (valve
#       960, HVS 1920), `no` (equal), `unknown` (anything else or a value
#       missing)
#   rpi5_dispregs verdict: q5 clock <name> (<id>): state <on|off|no clock|not answered> rate <hz|-> measured <hz|-> max <hz|->
#   rpi5_dispregs verdict: q5: <answer>   the disp (16), hsm (13) and bvb (14) clocks
#   rpi5_dispregs verdict: q6: <answer>   display count, settings and the HDMI words
#   rpi5_dispregs verdict: end: <the image's end line, or missing>
#   rpi5_dispregs verdict: done | FAIL (<reason>) | incomplete (<reason>)
#
# The image's lines (all prefixed `rpi5_dispregs: `; others are ignored):
#
#   hvs ch<n> ctrl0 <hex> enb <0|1> fwidth <n> lines <n> status <hex> mode <n> frcnt <n>
#   pv<n> decode en <0|1> viden <0|1> clk_select <n> pixel_rep <n> format <n> hactive <n> ... vactive <n> ... vic <n>
#   mbox <state|rate|measured|max> tag <hex> clock <id> status <hex> answered <hex> value <hex> rc <n> stage <s>
#   mbox num_displays tag <hex> status <hex> answered <hex> value <hex> rc <n> stage <s>
#   mbox display_settings tag <hex> ... rc <n> stage <s>
#   hdmi skipped (hsm clock off)  |  hdmi reading (...) then
#   hdmi<0|1> hotplug <hex> connected <0|1> hd_vid_ctl <hex> enable <0|1>
#   done (...) | FAIL (<reason>)
#
# A mailbox tag is answered when its rc is 0 (response code 0x80000000 and
# the tag's answered bit 31, checked by the image). Clock state: bit 0 on,
# bit 1 no such clock (fw_clk.h).
#
# Exit status: 0 the image said done, 1 incomplete (no end line), 2 usage,
# 3 the image said FAIL.
#
# Self-test: tools/probes/rpi_5/rpi5_dispregs/verdicts_selftest.sh
set -u

if [ $# -ne 1 ]; then
    echo "usage: ${0##*/} <console.log>" >&2
    exit 2
fi
if [ ! -r "$1" ] || [ -d "$1" ]; then
    echo "${0##*/}: cannot read $1" >&2
    exit 2
fi

# Consoles carry CR line ends; strip them before awk sees the fields.
tr -d '\r' <"$1" | awk '
function hexval(s,    i, c, v, d) {
    v = 0
    s = tolower(s)
    if (substr(s, 1, 2) != "0x")
        return -1
    for (i = 3; i <= length(s); i++) {
        c = substr(s, i, 1)
        d = index("0123456789abcdef", c)
        if (d == 0)
            return -1
        v = v * 16 + d - 1
    }
    return v
}
# the value after the key word `k` on the current line, or ""
function field(k,    i) {
    for (i = 1; i < NF; i++)
        if ($i == k)
            return $(i + 1)
    return ""
}
function num(s) { return s == "" ? "-" : sprintf("%.0f", s + 0) }
BEGIN {
    nc = split("4 13 14 16", cid, " ")
    split("core hsm bvb disp", cname, " ")
    nt = split("state rate measured max", tname, " ")
    endline = ""
}
{
    sub(/^.*rpi5_dispregs: /, "rpi5_dispregs: ")
}
/^rpi5_dispregs: (done|FAIL) \(/ {
    endline = $0
    sub(/^rpi5_dispregs: /, "", endline)
    next
}
/^rpi5_dispregs: hvs ch[01] / {
    ch = substr($3, 3, 1)
    hvs_seen[ch] = 1
    hvs_fw[ch] = field("fwidth")
    hvs_lines[ch] = field("lines")
    hvs_enb[ch] = field("enb")
    next
}
/^rpi5_dispregs: pv[01] decode / {
    n = substr($2, 3, 1)
    pv_seen[n] = 1
    pv_en[n] = field("en")
    pv_viden[n] = field("viden")
    pv_clk[n] = field("clk_select")
    pv_rep[n] = field("pixel_rep")
    pv_w[n] = field("hactive")
    pv_h[n] = field("vactive")
    pv_vic[n] = field("vic")
    next
}
/^rpi5_dispregs: mbox (state|rate|measured|max) tag / {
    k = $3 SUBSEP field("clock")
    seen[k] = 1
    ok[k] = (field("rc") == "0")
    val[k] = field("value")
    next
}
/^rpi5_dispregs: mbox num_displays / {
    nd_seen = 1
    nd_ok = (field("rc") == "0")
    nd_val = field("value")
    next
}
/^rpi5_dispregs: mbox display_settings / {
    ds_seen = 1
    ds_ok = (field("rc") == "0")
    next
}
/^rpi5_dispregs: hdmi skipped/ {
    hdmi = "skipped"
    next
}
/^rpi5_dispregs: hdmi reading/ {
    hdmi = "read"
    next
}
/^rpi5_dispregs: hdmi[01] hotplug / {
    p = substr($2, 5, 1)
    hp_seen[p] = 1
    hp[p] = field("connected")
    hv_en[p] = field("enable")
    next
}
END {
    # ---- Q3 ----
    nrun = 0
    first = ""
    for (n = 0; n <= 1; n++) {
        if (!(n in pv_seen)) {
            printf "rpi5_dispregs verdict: q3 pv%d: not read (no decode line in the log)\n", n
            continue
        }
        run = (pv_en[n] == "1" && pv_viden[n] == "1")
        fw = (n in hvs_seen) ? hvs_fw[n] : ""
        printf "rpi5_dispregs verdict: q3 pv%d: %s clk_select %s pixel_rep %s active %sx%s vic %s; hvs ch%d fwidth %s lines %s\n", n, run ? "running" : "stopped", pv_clk[n], pv_rep[n], pv_w[n], pv_h[n], pv_vic[n], n, fw == "" ? "-" : fw, (n in hvs_seen) ? hvs_lines[n] : "-"
        if (run) {
            nrun++
            if (first == "")
                first = n
        }
    }
    if (first == "") {
        if (!(0 in pv_seen) && !(1 in pv_seen))
            q3 = "unknown (no pixel valve line in the log)"
        else
            q3 = "no pixel valve runs (EN and VIDEN set on neither), so no valve feeds HDMI0 now"
    } else {
        n = first
        fw = (n in hvs_seen) ? hvs_fw[n] + 0 : 0
        w = pv_w[n] + 0
        if (fw == 0 || w == 0)
            ppc = "unknown (valve width " num(pv_w[n]) ", HVS FWIDTH " ((n in hvs_seen) ? num(hvs_fw[n]) : "missing") ")"
        else if (w * 2 == fw)
            ppc = "yes (valve active width " w " = HVS FWIDTH " fw " / 2)"
        else if (w == fw)
            ppc = "no (valve active width " w " = HVS FWIDTH " fw ")"
        else
            ppc = "unknown (valve active width " w ", HVS FWIDTH " fw ")"
        q3 = "pv" n " runs" (nrun > 1 ? " (both valves run)" : "") " (the repo maps pv0 to HDMI0, pv1 to HDMI1); two pixels per clock: " ppc
    }
    printf "rpi5_dispregs verdict: q3: %s\n", q3

    # ---- Q5 ----
    for (i = 1; i <= nc; i++) {
        id = cid[i]
        ks = "state" SUBSEP id
        if (!(ks in seen))
            st = "not answered (no line in the log)"
        else if (!ok[ks])
            st = "not answered"
        else {
            sv = hexval(val[ks])
            if (sv < 0)
                st = "not answered"
            else if (int(sv / 2) % 2 == 1)
                st = "no clock"
            else if (sv % 2 == 1)
                st = "on"
            else
                st = "off"
        }
        state[id] = st
        line = ""
        for (t = 2; t <= nt; t++) {
            k = tname[t] SUBSEP id
            v = "-"
            if ((k in seen) && ok[k] && hexval(val[k]) >= 0)
                v = sprintf("%.0f", hexval(val[k]))
            line = line " " tname[t] " " v
            if (t == 3)
                meas[id] = v
        }
        printf "rpi5_dispregs verdict: q5 clock %s (%d): state %s%s\n", cname[i], id, st, line
    }
    q5 = ""
    for (i = 1; i <= nc; i++) {
        id = cid[i]
        if (id == 4)
            continue
        q5 = q5 (q5 == "" ? "" : ", ") cname[i] " " state[id] " measured " meas[id]
    }
    printf "rpi5_dispregs verdict: q5: %s\n", q5

    # ---- Q6 ----
    if (!nd_seen)
        nds = "displays: missing (no line in the log)"
    else if (!nd_ok)
        nds = "displays: not answered"
    else
        nds = "displays: " (hexval(nd_val) < 0 ? "unparsable" : sprintf("%.0f", hexval(nd_val)))
    if (!ds_seen)
        dss = "settings: missing"
    else
        dss = "settings: " (ds_ok ? "answered" : "not answered")
    if (hdmi == "skipped")
        hs = "hdmi: skipped (hsm clock off)"
    else if (hdmi == "read") {
        hs = "hdmi:"
        for (p = 0; p <= 1; p++) {
            if (p in hp_seen)
                hs = hs " hdmi" p " hotplug " hp[p] " hd_vid_ctl enable " hv_en[p] (p == 0 ? "," : "")
            else
                hs = hs " hdmi" p " not read" (p == 0 ? "," : "")
        }
        if (0 in hp_seen) {
            if (hp[0] == "1" && hv_en[0] == "1")
                hs = hs "; the firmware drives HDMI0"
            else if ((1 in hp_seen) && hp[1] == "1" && hv_en[1] == "1")
                hs = hs "; the firmware drives HDMI1"
            else
                hs = hs "; no port shows a sink with the encoder on"
        }
    } else
        hs = "hdmi: missing (no hdmi line in the log)"
    printf "rpi5_dispregs verdict: q6: %s, %s, %s\n", nds, dss, hs

    # ---- end ----
    if (endline != "")
        printf "rpi5_dispregs verdict: end: %s\n", endline
    else
        printf "rpi5_dispregs verdict: end: missing (the log stops before the image'"'"'s end line)\n"
    if (endline ~ /^done \(/) {
        print "rpi5_dispregs verdict: done"
        exit 0
    }
    if (endline ~ /^FAIL \(/) {
        r = endline
        sub(/^FAIL \(/, "", r)
        sub(/\)$/, "", r)
        printf "rpi5_dispregs verdict: FAIL (%s)\n", r
        exit 3
    }
    print "rpi5_dispregs verdict: incomplete (no end line)"
    exit 1
}'
