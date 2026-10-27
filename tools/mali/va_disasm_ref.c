// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Mesa 26.2.1's Valhall disassembler on a list of words: the reference text
// for src/shaderc/online/mali/isa/tests.rs. Built by tools/mali/va_disasm_ref.sh
// with valhall_disasm.c, which Mesa's own disasm.py generates from the
// vendored ISA.xml. Input lines are "xx xx xx xx xx xx xx xx    text" (the
// 8 bytes of a word in memory order, as pandecode and Mesa's
// assembler-cases.txt write them) or "#" comments; each word line is written
// back with Mesa's text in place of the old one, each comment unchanged.

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "disassemble.h"

int
main(void)
{
   char line[512];
   unsigned words = 0;

   while (fgets(line, sizeof(line), stdin) != NULL) {
      size_t len = strlen(line);
      if (len == 0 || line[len - 1] != '\n') {
         fprintf(stderr, "va_disasm_ref: a line without a newline or too long\n");
         return 1;
      }
      if (line[0] == '#') {
         fputs(line, stdout);
         continue;
      }
      uint64_t w = 0;
      for (unsigned i = 0; i < 8; ++i) {
         unsigned b;
         if (sscanf(line + 3 * i, "%2x", &b) != 1) {
            fprintf(stderr, "va_disasm_ref: not a word: %s", line);
            return 1;
         }
         w |= (uint64_t)b << (8 * i);
      }
      fwrite(line, 1, 27, stdout);
      va_disasm_instr(stdout, w);
      fputc('\n', stdout);
      words++;
   }
   if (words == 0) {
      fprintf(stderr, "va_disasm_ref: no word\n");
      return 1;
   }
   return 0;
}
