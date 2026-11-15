"""Достаёт из экспорта спецификации все фрагменты в обратных кавычках.
Запуск: python3 -I spans.py <spec.md> <out.tsv>
"""
import re, sys
src, out = sys.argv[1], sys.argv[2]
lines = open(src, encoding='utf-8').read().split('\n')
rows = []
for i, l in enumerate(lines, 1):
    for m in re.finditer(r'`([^`]*)`', l):
        rows.append((i, m.group(1)))
with open(out, 'w', encoding='utf-8') as f:
    for i, s in rows:
        f.write(f"{i}\t{s}\n")
print("фрагментов:", len(rows), "уникальных:", len(set(s for _, s in rows)))
