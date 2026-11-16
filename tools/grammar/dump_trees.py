"""Записывает эталонные деревья: как проверочный разборщик разбирает все примеры.

По этому файлу сверяется разборщик на Rust (compiler/tests/parser_golden.rs).
Запуск: python3 tools/grammar/dump_trees.py grammar > grammar/trees.golden.tsv
"""
import glob
import os
import sys

from canon import Reference, esc

G = sys.argv[1]
ref = Reference(G)


def rows(name):
    for line in open(os.path.join(G, name), encoding="utf-8"):
        line = line.rstrip("\n")
        if line and not line.startswith("#"):
            yield line.split("\t")


cases = []          # (имя, стартовое правило, текст, ожидаемый исход)
number = 0
for row in rows("fragments.tsv"):
    orig, start, text = (row + ["", ""])[:3]
    if start == "-":
        continue
    number += 1
    cases.append((f"frag:{number}", start, (text or orig).replace("\\n", "\n"), "OK"))
number = 0
for row in rows("negative.tsv"):
    number += 1
    cases.append((f"neg:{number}", row[0], row[1].replace("\\n", "\n"), "ERROR"))
for path in sorted(glob.glob(os.path.join(G, "samples", "*.nx"))):
    cases.append(("sample:" + os.path.basename(path), "file", open(path, encoding="utf-8").read(), "OK"))

print("# эталонные деревья: пример <TAB> стартовое правило <TAB> текст <TAB> OK или ERROR <TAB> запись дерева")
wrong = 0
for case_id, start, text, expected in cases:
    status, tree = ref.run(start, text)
    if status != expected:
        wrong += 1
        print(f"{case_id}: ожидалось {expected}, получено {status}", file=sys.stderr)
    print("\t".join([case_id, start, esc(text), status] + ([esc(tree)] if tree is not None else [])))
if wrong:
    sys.exit(1)
