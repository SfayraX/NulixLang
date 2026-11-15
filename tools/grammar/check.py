"""Проверка черновика грамматики Nulix.

1. Строит LALR(1)-разборщик в строгом режиме: любой конфликт — ошибка.
2. Разбирает фрагменты из спецификации (grammar/fragments.tsv).
3. Проверяет, что каждый фрагмент раздела «Синтаксис» учтён.
4. Разбирает тестовые программы (grammar/samples/*.nx) и отрицательные примеры (grammar/negative.tsv).
Запуск: python3 check.py <папка grammar> [-v]
"""
import glob, os, re, sys
from lark import Lark
from lark.exceptions import LarkError
import nxlex

G = sys.argv[1]
VERBOSE = "-v" in sys.argv
STARTS = ["file", "decl", "stmt", "stmts", "expr", "type", "pattern", "func_sig", "generics", "generic_args",
          "reqs", "match_item", "import_decl", "attr", "sig_tail", "param", "field", "variant", "bound",
          "file_header", "member"]
grammar = open(os.path.join(G, "nulix.lark"), encoding="utf-8").read()
try:
    parser = Lark(grammar, parser="lalr", lexer=nxlex.NxLexer, start=STARTS, strict=True)
except LarkError as e:
    print("ГРАММАТИКА НЕ СОБРАНА:\n" + str(e)[:3000])
    sys.exit(1)
rules = {r.origin.name for r in parser.rules if not str(r.origin.name).startswith("__")}
lits = sorted(t.pattern.value for t in parser.terminals if t.pattern is not None)
words = sorted(v for v in lits if v[0].isalpha())
print(f"грамматика собрана без конфликтов: правил {len(rules)}, ключевых слов {len(words)}, "
      f"знаков и пометок {len(lits) - len(words)}")


def parse(text, start):
    """Возвращает None при успехе или текст ошибки."""
    nxlex.EMBEDDED.clear()
    nxlex.EOF_TERM = start in ("file", "stmts")
    try:
        parser.parse(text, start=start)
        for line, sub in list(nxlex.EMBEDDED):
            nxlex.EOF_TERM = False
            parser.parse(sub, start="expr")
    except (LarkError, nxlex.NxLexError) as e:
        return " ".join(str(e).split())[:200]
    return None


# ── 2. фрагменты ──
table, skip = {}, {}
for ln in open(os.path.join(G, "fragments.tsv"), encoding="utf-8"):
    ln = ln.rstrip("\n")
    if not ln or ln.startswith("#"):
        continue
    orig, start, text = (ln.split("\t") + ["", ""])[:3]
    if start == "-":
        skip[orig] = text
    else:
        table[orig] = (start, text or orig)
bad = []
for orig, (start, text) in table.items():
    err = parse(text.replace("\\n", "\n"), start)
    if err:
        bad.append((orig, start, text, err))
print(f"фрагменты спецификации: разобрано {len(table) - len(bad)} из {len(table)}; пропущено с причиной {len(skip)}")
for orig, start, text, err in bad:
    print(f"  НЕ РАЗОБРАН [{start}] {orig!r} как {text!r}: {err}")

# ── 3. полнота: все фрагменты раздела «Синтаксис» (строки 354–486 экспорта) ──
lexer = nxlex.NxLexer.__new__(nxlex.NxLexer)
lexer.lit = {t.pattern.value: t.name for t in parser.terminals if t.pattern is not None}
lexer.words = set(words)
lexer.ops = sorted((v for v in lexer.lit if not v[0].isalpha() and v[0] not in "@_"), key=len, reverse=True)
seen, single, missing = set(), {}, []
for ln in open(os.path.join(G, "spans.tsv"), encoding="utf-8"):
    num, frag = ln.rstrip("\n").split("\t", 1)
    if not (354 <= int(num) <= 486) or frag in seen:
        continue
    seen.add(frag)
    if frag in table or frag in skip:
        continue
    try:
        nxlex.EOF_TERM = False
        toks = list(lexer.lex(frag))
    except nxlex.NxLexError:
        toks = []
    if len(toks) == 1:
        single[frag] = "имя" if toks[0].type == "NAME" else ("ключевое слово" if frag in lexer.words else "знак")
    else:
        missing.append(frag)
kw_in_spec = sorted(f for f, k in single.items() if k == "ключевое слово")
print(f"раздел «Синтаксис»: разных фрагментов {len(seen)}; одиночных лексем {len(single)} "
      f"(ключевых слов {len(kw_in_spec)}); не учтено {len(missing)}")
for f in missing:
    print(f"  НЕ УЧТЁН: {f!r}")
if VERBOSE:
    print("  ключевые слова, встреченные как отдельные фрагменты:", " ".join(kw_in_spec))
    print("  все ключевые слова грамматики:", " ".join(words))
    print("  имена:", " ".join(sorted(f for f, k in single.items() if k == "имя")))

# ── 4. тестовые программы и отрицательные примеры ──
ok = 0
files = sorted(glob.glob(os.path.join(G, "samples", "*.nx")))
for path in files:
    err = parse(open(path, encoding="utf-8").read(), "file")
    ok += err is None
    if err:
        print(f"  ПРОГРАММА НЕ РАЗОБРАНА {os.path.basename(path)}: {err}")
print(f"тестовые программы: разобрано {ok} из {len(files)}")
neg, wrong = 0, []
npath = os.path.join(G, "negative.tsv")
if os.path.exists(npath):
    for ln in open(npath, encoding="utf-8"):
        ln = ln.rstrip("\n")
        if not ln or ln.startswith("#"):
            continue
        start, text = ln.split("\t")[:2]
        neg += 1
        if parse(text.replace("\\n", "\n"), start) is None:
            wrong.append((start, text))
    print(f"отрицательные примеры: отвергнуто {neg - len(wrong)} из {neg}")
    for start, text in wrong:
        print(f"  ОШИБОЧНО ПРИНЯТ [{start}] {text!r}")
