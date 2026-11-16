"""Тексты у предела вложенности — для сверки разборщика на Rust с проверочным.

Для каждого способа вложить одно в другое строится текст с глубиной чуть меньше предела, ровно
на пределе и чуть больше; затем — случайные смеси способов. Исход каждого текста и запись дерева
берутся у проверочного разборщика. Разборщик на Rust обязан выдать то же самое. Деревья здесь
большие, поэтому вместо записи дерева в файл идёт её контрольная сумма (FNV-1a, 64 бита) после «#».

Запуск: python3 tools/grammar/depth_cases.py grammar > grammar/depth.golden.tsv
"""
import random
import sys

from canon import MAX_DEPTH, Reference, esc

ref = Reference(sys.argv[1])
rnd = random.Random(1)

# стартовое правило и текст с n уровнями вложенности
KINDS = [
    ("круглые скобки", "expr", lambda n: "(" * n + "1" + ")" * n),
    ("списки", "expr", lambda n: "[" * n + "]" * n),
    ("вызовы", "expr", lambda n: "f(" * n + ")" * n),
    ("индексы", "expr", lambda n: "a[" * n + "0" + "]" * n),
    ("блоки if", "expr", lambda n: "if c { " * n + "}" * n),
    ("блоки замыканий", "expr", lambda n: "func() { " * n + "}" * n),
    ("блоки trust", "expr", lambda n: 'trust "r" { ' * n + "}" * n),
    ("блоки уровня", "expr", lambda n: "@low { " * n + "}" * n),
    ("блоки catch", "expr", lambda n: "x catch { " * n + "}" * n),
    ("match в match", "expr", lambda n: "match x { case _ { " * (n // 2) + "(" * (n % 2) + "1" + ")" * (n % 2)
                                        + " } }" * (n // 2)),
    ("target в target", "expr", lambda n: "target { case a { " * (n // 2) + "(" * (n % 2) + "1" + ")" * (n % 2)
                                          + " } }" * (n // 2)),
    ("not", "expr", lambda n: "not " * n + "x"),
    ("минус", "expr", lambda n: "- " * n + "x"),
    ("тильда", "expr", lambda n: "~ " * n + "x"),
    ("try", "expr", lambda n: "try " * n + "x"),
    ("await", "expr", lambda n: "await " * n + "x"),
    ("not и минус", "expr", lambda n: "not " * (n // 2) + "- " * (n - n // 2) + "x"),
    ("if в условии if", "expr", lambda n: "if " * n + "c" + " { }" * n),
    ("match в заголовке match", "expr", lambda n: "match " * n + "x" + " { }" * n),
    ("provide в значении provide", "expr", lambda n: "provide a = " * n + "x" + " { }" * n),
    ("замыкание в limit", "expr", lambda n: "func() limit a <= " * n + "1" + " { }" * n),
    ("условие в case", "expr", lambda n: "match x { case _ if " + "(" * (n - 2) + "c" + ")" * (n - 2) + " { } }"),
    ("образец в is", "expr", lambda n: "x is " + "A(" * n + "_" + ")" * n),
    ("подстановка в строке", "expr", lambda n: "(" * (n - 1) + '"${x}"' + ")" * (n - 1)),
    ("строка в строке", "expr", lambda n: "(" * (n - 2) + '"""${ "${ x }" }"""' + ")" * (n - 2)),
    ("скобки в подстановке", "expr", lambda n: '"${' + "(" * (n - 1) + "x" + ")" * (n - 1) + '}"'),
    ("ref", "type", lambda n: "ref " * n + "T"),
    ("mut ref", "type", lambda n: "mut ref " * n + "T"),
    ("ptr", "type", lambda n: "ptr " * n + "T"),
    ("mut ptr", "type", lambda n: "mut ptr " * n + "T"),
    ("ref с источником", "type", lambda n: "ref(a) " * (n - 1) + "T"),
    ("типы результата", "type", lambda n: "func() -> " * n + "T"),
    ("параметры типов функций", "type", lambda n: "func(" * n + ")" * n),
    ("аргументы типов", "type", lambda n: "A[" * n + "T" + "]" * n),
    ("кортежи типов", "type", lambda n: "(T, " * n + "T" + ")" * n),
    ("образцы-конструкторы", "pattern", lambda n: "A(" * n + "_" + ")" * n),
    ("образцы-кортежи", "pattern", lambda n: "(_, " * n + "_" + ")" * n),
    ("заголовок for", "stmt", lambda n: "for i in " + "(" * (n - 1) + "x" + ")" * (n - 1) + " { }"),
    ("заголовок while", "stmt", lambda n: "while " + "(" * (n - 1) + "x" + ")" * (n - 1) + " { }"),
    ("результат функции", "decl", lambda n: "func f() -> " + "(" * (n - 1) + "T" + ")" * (n - 1) + " { }"),
    ("тип поля", "decl", lambda n: "struct S { a: " + "A[" * (n - 1) + "T" + "]" * (n - 1) + " }"),
    ("аргументы пометки", "decl", lambda n: "@a" + "(" * n + "1" + ")" * n + " func f() { }"),
    ("целый файл", "file", lambda n: "x = " + "(" * n + "1" + ")" * n + "\n"),
]

# способы обернуть выражение ещё одним уровнем — для случайных смесей
WRAPS = [
    lambda e: "(" + e + ")", lambda e: "[" + e + "]", lambda e: "f(" + e + ")", lambda e: "a[" + e + "]",
    lambda e: "g(x, " + e + ")", lambda e: "h(n: " + e + ")", lambda e: "- " + e, lambda e: "try " + e,
    lambda e: "if c { " + e + " }", lambda e: "func() { " + e + " }", lambda e: "@low { " + e + " }",
    lambda e: "match " + e + " { }", lambda e: "provide a = " + e + " { }",
]

print(f"# тексты у предела вложенности ({MAX_DEPTH}): пример <TAB> стартовое правило <TAB> текст "
      f"<TAB> OK или ERROR <TAB> # и контрольная сумма записи дерева")
count = {"OK": 0, "ERROR": 0}


def checksum(text):
    value = 0xCBF29CE484222325
    for byte in text.encode("utf-8"):
        value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"#{value:016x}"


def emit(case_id, start, text):
    status, tree = ref.run(start, text)
    count[status] += 1
    print("\t".join([case_id, start, esc(text), status] + ([checksum(tree)] if tree is not None else [])))
    return status


unexpected = []
for number, (name, start, make) in enumerate(KINDS, 1):
    for n in (MAX_DEPTH - 1, MAX_DEPTH, MAX_DEPTH + 1):
        status = emit(f"depth:{number}:{n}", start, make(n))
        if status != ("OK" if n <= MAX_DEPTH else "ERROR"):
            unexpected.append(f"{name}, {n} уровней: {status}")
for number in range(1, 13):
    for n in (MAX_DEPTH - 1, MAX_DEPTH, MAX_DEPTH + 1):
        text = "1"
        for _ in range(n):
            text = rnd.choice(WRAPS)(text)
        status = emit(f"mix:{number}:{n}", "expr", text)
        if status != ("OK" if n <= MAX_DEPTH else "ERROR"):
            unexpected.append(f"смесь {number}, {n} уровней: {status}")
print(f"текстов {count['OK'] + count['ERROR']}: принято {count['OK']}, отвергнуто {count['ERROR']}", file=sys.stderr)
for line in unexpected:
    print("НЕ ПО ОЖИДАНИЮ: " + line, file=sys.stderr)
if unexpected:
    sys.exit(1)
