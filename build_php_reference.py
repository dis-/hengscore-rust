"""Reproducible data extraction from pinned PHP libmbfl sources (see NOTICE)."""
from pathlib import Path
import hashlib
import json
import re
import struct

root = Path(__file__).parent
ref = root / 'php-reference'
rust_data = root / 'src' if (root / 'src').is_dir() else root
header = (ref / 'unicode_table_jis.h').read_text(encoding='utf-8')
old = (ref / 'unicode_table_jis-7.4.h').read_text(encoding='utf-8')
code = (ref / 'mbfilter_cjk.c').read_text(encoding='utf-8')

def array(text, name):
    body = re.search(r'\b' + name + r'\[\]\s*=\s*\{(.*?)\};', text, re.S)[1]
    body = re.sub(r'/\*.*?\*/', '', body, flags=re.S)
    return [int(x.strip(), 0) for x in body.split(',') if x.strip()]

report = {}
for short in ('0208', '0212'):
    name = f'jisx{short}_ucs_table'
    values = array(header, name)
    assert values == array(old, name), f'{name} differs between PHP 7.4 and 8.4'
    assert all(0 <= x <= 0xffff and not 0xd800 <= x <= 0xdfff for x in values)
    data = struct.pack('<' + 'H' * len(values), *values)
    (rust_data / f'jis{short}.bin').write_bytes(data)
    report[name] = dict(entries=len(values), php_7_4_identical=True,
                        sha256=hashlib.sha256(data).hexdigest())

def function(name):
    start = code.index('static size_t ' + name + '(')
    brace = code.index('{', start)
    depth = 1
    end = brace + 1
    while depth:
        depth += (code[end] == '{') - (code[end] == '}')
        end += 1
    return code[start:end]

tables = '\n'.join(re.search(r'static const unsigned short ' + name + r'\[\].*?\};', code, re.S)[0]
                   for name in ('sjis_decode_tbl1', 'sjis_decode_tbl2'))
preamble = '''/* Test-only harness of the PHP libmbfl decoder functions. See NOTICE. */
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include "unicode_table_jis.h"
#define MBFL_BAD_INPUT 0xffffffffU
'''
main = r'''
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
'''
(ref / 'decoder_oracle.c').write_text(preamble + tables + '\n' + function('mb_sjis_to_wchar')
                                  + '\n' + function('mb_eucjp_to_wchar') + main, encoding='utf-8')
report['source_files'] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                          for p in (ref / 'unicode_table_jis.h', ref / 'mbfilter_cjk.c', ref / 'unicode_table_jis-7.4.h')}
(ref / 'extraction_report.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
