"""Записывает эталонные лексемы: что выдаёт проверочный лексер на всех примерах.

По этому файлу сверяется лексер на Rust (compiler/tests/lexer_golden.rs).
Запуск: python3 tools/grammar/dump_tokens.py grammar > grammar/tokens.golden.tsv
"""
import glob
import os
import sys

from lark import Lark

import nxlex

G = sys.argv[1]
grammar = open(os.path.join(G, "nulix.lark"), encoding="utf-8").read()
parser = Lark(grammar, parser="lalr", lexer=nxlex.NxLexer, start=["file"], strict=True)
lexer = nxlex.NxLexer.__new__(nxlex.NxLexer)
lexer.lit = {t.pattern.value: t.name for t in parser.terminals if t.pattern is not None}
lexer.words = {v for v in lexer.lit if v[0].isalpha()}
lexer.ops = sorted((v for v in lexer.lit if not v[0].isalpha() and v[0] not in "@_"), key=len, reverse=True)


def esc(text):
    return text.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t").replace("\r", "\\r")


def dump(token):
    if token.type in ("NAME", "INT", "FLOAT", "STRING", "CHAR", "ATTR"):
        return f"{token.type}:{token.value}"
    return "TERM" if token.type == "TERM" else "kw:" + token.value


def rows(name):
    for line in open(os.path.join(G, name), encoding="utf-8"):
        line = line.rstrip("\n")
        if line and not line.startswith("#"):
            yield line.split("\t")


cases = []
number = 0
for row in rows("fragments.tsv"):
    orig, start, text = (row + ["", ""])[:3]
    if start == "-":
        continue
    number += 1
    cases.append((f"frag:{number}", start in ("file", "stmts"), (text or orig).replace("\\n", "\n")))
number = 0
for row in rows("negative.tsv"):
    number += 1
    cases.append((f"neg:{number}", row[0] in ("file", "stmts"), row[1].replace("\\n", "\n")))
for path in sorted(glob.glob(os.path.join(G, "samples", "*.nx"))):
    cases.append(("sample:" + os.path.basename(path), True, open(path, encoding="utf-8").read()))

print("# эталонные лексемы: пример <TAB> конец инструкции в конце текста <TAB> текст <TAB> OK или ERROR <TAB> лексемы")
for case_id, eof_term, text in cases:
    nxlex.EOF_TERM = eof_term
    nxlex.EMBEDDED.clear()
    head = [case_id, "1" if eof_term else "0", esc(text)]
    try:
        print("\t".join(head + ["OK"] + [esc(dump(t)) for t in lexer.lex(text)]))
    except nxlex.NxLexError:
        print("\t".join(head + ["ERROR"]))
