// SPDX-License-Identifier: GPL-3.0-or-later
//
// Minimal PCRE2 (8-bit) declarations, only for regen.sh when the libpcre2-dev
// headers are not installed: they let the unmodified Gource core/regex.cpp be
// compiled against the system libpcre2-8.so.0.
#pragma once
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct pcre2_real_code_8 pcre2_code;
typedef struct pcre2_real_match_data_8 pcre2_match_data;
typedef struct pcre2_real_general_context_8 pcre2_general_context;
typedef struct pcre2_real_compile_context_8 pcre2_compile_context;
typedef struct pcre2_real_match_context_8 pcre2_match_context;
typedef const uint8_t *PCRE2_SPTR;
typedef size_t PCRE2_SIZE;
pcre2_code *pcre2_compile_8(PCRE2_SPTR, PCRE2_SIZE, uint32_t, int *, PCRE2_SIZE *, pcre2_compile_context *);
pcre2_code *pcre2_code_copy_8(const pcre2_code *);
void pcre2_code_free_8(pcre2_code *);
pcre2_match_data *pcre2_match_data_create_from_pattern_8(const pcre2_code *, pcre2_general_context *);
int pcre2_match_8(const pcre2_code *, PCRE2_SPTR, PCRE2_SIZE, PCRE2_SIZE, uint32_t, pcre2_match_data *, pcre2_match_context *);
PCRE2_SIZE *pcre2_get_ovector_pointer_8(pcre2_match_data *);
void pcre2_match_data_free_8(pcre2_match_data *);
#ifdef __cplusplus
}
#endif
#define pcre2_compile pcre2_compile_8
#define pcre2_code_copy pcre2_code_copy_8
#define pcre2_code_free pcre2_code_free_8
#define pcre2_match_data_create_from_pattern pcre2_match_data_create_from_pattern_8
#define pcre2_match pcre2_match_8
#define pcre2_get_ovector_pointer pcre2_get_ovector_pointer_8
#define pcre2_match_data_free pcre2_match_data_free_8
