/* Test-only harness of the PHP libmbfl decoder functions. See NOTICE. */
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include "unicode_table_jis.h"
#define MBFL_BAD_INPUT 0xffffffffU
static const unsigned short sjis_decode_tbl1[] = {
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFFFF, 0, 188, 376, 564, 752, 940, 1128, 1316, 1504, 1692, 1880, 2068, 2256, 2444, 2632, 2820, 3008, 3196, 3384, 3572, 3760, 3948, 4136, 4324, 4512, 4700, 4888, 5076, 5264, 5452, 5640, 0xFFFF, -6016, -5828, -5640, -5452, -5264, -5076, -4888, -4700, -4512, -4324, -4136, -3948, -3760, -3572, -3384, -3196, -3008, -2820, -2632, -2444, -2256, -2068, -1880, -1692, -1504, -1316, -1128, -940, -752, -564, -376, -188, 0, 188, 376, 564, 752, 940, 1128, 1316, 1504, 1692, 1880, 2068, 2256, 2444, 2632, 2820, 3008, 3196, 3384, 3572, 3760, 3948, 4136, 4324, 4512, 4700, 4888, 5076, 5264, 5452, 5640, 5828, 6016, 6204, 6392, 6580, 6768, 6956, 7144, 7332, 7520, 7708, 7896, 8084, 8272, 8460, 8648, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF
};
static const unsigned short sjis_decode_tbl2[] = {
	0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 0xFFFF, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 0xFFFF, 0xFFFF, 0xFFFF
};
static size_t mb_sjis_to_wchar(unsigned char **in, size_t *in_len, uint32_t *buf, size_t bufsize, unsigned int *state)
{
	unsigned char *p = *in, *e = p + *in_len;
	uint32_t *out = buf, *limit = buf + bufsize;

	e--; /* Stop the main loop 1 byte short of the end of the input */

	while (p < e && out < limit) {
		unsigned char c = *p++;

		if (c <= 0x7F) {
			*out++ = c;
		} else if (c >= 0xA1 && c <= 0xDF) { /* Kana */
			*out++ = 0xFEC0 + c;
		} else {
			/* Don't need to check p < e; it's not possible to go out of bounds here, due to e-- above */
			unsigned char c2 = *p++;
			/* This is only legal if c2 >= 0x40 && c2 <= 0xFC && c2 != 0x7F
			 * But the values in the above conversion tables have been chosen such that
			 * illegal values of c2 will always result in w > jisx0208_ucs_table_size,
			 * so we don't need to do a separate bounds check on c2
			 * Likewise, the values in the conversion tables are such that illegal values
			 * for c will always result in w > jisx0208_ucs_table_size */
			uint32_t w = sjis_decode_tbl1[c] + sjis_decode_tbl2[c2];
			if (w < jisx0208_ucs_table_size) {
				w = jisx0208_ucs_table[w];
				if (!w)
					w = MBFL_BAD_INPUT;
				*out++ = w;
			} else {
				if (c == 0x80 || c == 0xA0 || c > 0xEF) {
					p--;
				}
				*out++ = MBFL_BAD_INPUT;
			}
		}
	}

	/* Finish up last byte of input string if there is one */
	if (p == e && out < limit) {
		unsigned char c = *p++;
		if (c <= 0x7F) {
			*out++ = c;
		} else if (c >= 0xA1 && c <= 0xDF) {
			*out++ = 0xFEC0 + c;
		} else {
			*out++ = MBFL_BAD_INPUT;
		}
	}

	*in_len = e - p + 1;
	*in = p;
	return out - buf;
}
static size_t mb_eucjp_to_wchar(unsigned char **in, size_t *in_len, uint32_t *buf, size_t bufsize, unsigned int *state)
{
	unsigned char *p = *in, *e = p + *in_len;
	uint32_t *out = buf, *limit = buf + bufsize;

	while (p < e && out < limit) {
		unsigned char c = *p++;

		if (c < 0x80) {
			*out++ = c;
		} else if (c >= 0xA1 && c <= 0xFE && p < e) {
			/* JISX 0208 */
			unsigned char c2 = *p++;
			if (c2 >= 0xA1 && c2 <= 0xFE) {
				unsigned int s = (c - 0xA1)*94 + c2 - 0xA1;
				if (s < jisx0208_ucs_table_size) {
					uint32_t w = jisx0208_ucs_table[s];
					if (!w)
						w = MBFL_BAD_INPUT;
					*out++ = w;
				} else {
					*out++ = MBFL_BAD_INPUT;
				}
			} else {
				*out++ = MBFL_BAD_INPUT;
			}
		} else if (c == 0x8E && p < e) {
			/* Kana */
			unsigned char c2 = *p++;
			*out++ = (c2 >= 0xA1 && c2 <= 0xDF) ? 0xFEC0 + c2 : MBFL_BAD_INPUT;
		} else if (c == 0x8F) {
			/* JISX 0212 */
			if ((e - p) >= 2) {
				unsigned char c2 = *p++;
				unsigned char c3 = *p++;
				if (c3 >= 0xA1 && c3 <= 0xFE && c2 >= 0xA1 && c2 <= 0xFE) {
					unsigned int s = (c2 - 0xA1)*94 + c3 - 0xA1;
					if (s < jisx0212_ucs_table_size) {
						uint32_t w = jisx0212_ucs_table[s];
						if (!w)
							w = MBFL_BAD_INPUT;
						*out++ = w;
					} else {
						*out++ = MBFL_BAD_INPUT;
					}
				} else {
					*out++ = MBFL_BAD_INPUT;
				}
			} else {
				*out++ = MBFL_BAD_INPUT;
				p = e; /* Jump to end of string */
			}
		} else {
			*out++ = MBFL_BAD_INPUT;
		}
	}

	*in_len = e - p;
	*in = p;
	return out - buf;
}
static void emit(unsigned char charset, unsigned char *raw, unsigned char n) {
    unsigned char *p = raw;
    size_t remain = n;
    unsigned int state = 0;
    uint32_t out[8];
    size_t count = charset == 1 ? mb_sjis_to_wchar(&p, &remain, out, 8, &state)
                               : mb_eucjp_to_wchar(&p, &remain, out, 8, &state);
    unsigned char length = (unsigned char)count;
    if (remain) length = 255;
    for (size_t i = 0; i < count; ++i) if (out[i] == MBFL_BAD_INPUT) length = 255;
    putchar(charset); putchar(n); fwrite(raw, 1, n, stdout); putchar(length);
    if (length != 255) {
        for (size_t i = 0; i < count; ++i) {
            for (unsigned int shift = 0; shift < 32; shift += 8) putchar((out[i] >> shift) & 255);
        }
    }
}
int main(void) {
    unsigned char raw[3];
    for (unsigned char charset = 1; charset <= 2; ++charset) {
        for (unsigned int a = 0; a < 256; ++a) {
            raw[0] = a; emit(charset, raw, 1);
            for (unsigned int b = 0; b < 256; ++b) {
                raw[1] = b; emit(charset, raw, 2);
            }
        }
    }
    raw[0] = 0x8f;
    for (unsigned int a = 0; a < 256; ++a) {
        for (unsigned int b = 0; b < 256; ++b) {
            raw[1] = a; raw[2] = b; emit(2, raw, 3);
        }
    }
    return ferror(stdout) ? 1 : 0;
}
