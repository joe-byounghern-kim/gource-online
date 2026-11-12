/* SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Prints the first 1000 values of glibc's rand() without srand(), the state
 * Gource 0.57 draws from (it calls srand only for --start-position random).
 * Its output is crates/gource-core/tests/golden/glibc_rand_1000.txt, which
 * gource-core's GlibcRand (src/rand.rs) must reproduce:
 *
 *   gcc -O2 -o /tmp/glibc-rand scripts/parity/glibc-rand.c
 *   /tmp/glibc-rand > crates/gource-core/tests/golden/glibc_rand_1000.txt
 */
#include <stdio.h>
#include <stdlib.h>

int main(void) {
    for (int i = 0; i < 1000; i++) printf("%d\n", rand());
    return 0;
}
