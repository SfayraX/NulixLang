"""Случайные тексты по грамматике и их порча — для сверки разборщика на Rust с проверочным.

Тексты выводятся из правил grammar/nulix.lark: сначала по одному тексту на каждое раскрытие каждого
правила, потом случайные. Часть текстов портится одной правкой (лишняя, пропущенная или переставленная
лексема, перевод строки не на месте). Каждый текст разбирает проверочный разборщик, и его исход с
записью дерева попадает в файл. Разборщик на Rust обязан выдать то же самое на каждом тексте.

Запуск: python3 tools/grammar/fuzz.py grammar <сколько текстов> <зерно> > файл.tsv
"""
import random
import sys

from canon import Reference, esc
import nxlex

sys.setrecursionlimit(20000)
G, COUNT, SEED = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
rnd = random.Random(SEED)
ref = Reference(G)

# ── правила грамматики после раскрытия повторов и необязательных частей ──
LITERAL = {t.name: t.pattern.value for t in ref.parser.terminals if t.pattern is not None}
by_origin = {}
for rule in ref.parser.rules:
    by_origin.setdefault(str(rule.origin.name), []).append(rule)

# самая короткая цепочка лексем для каждого правила — чтобы вывод всегда завершался
INF = 10 ** 9
shortest = {name: INF for name in by_origin}


def rule_cost(rule):
    return sum(1 if s.is_term else shortest[str(s.name)] for s in rule.expansion)


changed = True
while changed:
    changed = False
    for name, rules in by_origin.items():
        best = min(min(rule_cost(r) for r in rules), INF)
        if best < shortest[name]:
            shortest[name], changed = best, True

SAMPLES = {
    "NAME": ["a", "b", "c", "x", "y", "n", "f", "g", "T", "U", "Item", "имя", "Список", "val_1",
             "type", "test", "life", "end", "discard"],
    "INT": ["0", "1", "7", "42", "0xFF", "0b101", "1_000"],
    "FLOAT": ["2.5", "1e9", "2.5e-3"],
    "STRING": ['"s"', '""', '"a $x b"', '"${a + b}"', '"\\$ \\n"', '"${f(1)} и $y"', '"""две\nстроки ${x}"""'],
    "CHAR": ["'c'", "'\\n'", "'я'"],
    "ATTR": ["@inline", "@align", "@test"],
}
NAME_WEIGHTS = [8] * 14 + [1] * 5      # слова, ключевые только на своём месте, — изредка
VOCAB = [("lit", v) for v in LITERAL.values()] + [(k, None) for k in SAMPLES] + [("TERM", None)]
used_rules = set()


def derive(name, state, out):
    """Случайный вывод из правила: пока есть запас — любое раскрытие, потом самое короткое."""
    rules = by_origin[name]
    fresh = [r for r in rules if r not in used_rules]
    if state["budget"] > 0 and fresh and rnd.random() < 0.5:
        rule = rnd.choice(fresh)          # чаще брать раскрытия, которых ещё не было в принятых текстах
    elif state["budget"] > 0:
        rule = rnd.choice(rules)
    else:
        low = min(rule_cost(r) for r in rules)
        rule = rnd.choice([r for r in rules if rule_cost(r) == low])
    state["budget"] -= 1
    state["rules"].append(rule)
    for s in rule.expansion:
        if s.is_term:
            out.append(token(str(s.name)))
        else:
            derive(str(s.name), state, out)


# как добраться до каждого правила от целого файла: правило-родитель и место в нём
via, queue = {}, ["file"]
while queue:
    name = queue.pop(0)
    for rule in by_origin[name]:
        for index, s in enumerate(rule.expansion):
            child = str(s.name)
            if not s.is_term and child != "file" and child not in via:
                via[child] = (rule, index)
                queue.append(child)


def derive_through(target, state, out):
    """Вывод целого файла, в котором обязательно применено раскрытие target."""
    chain, name = [(target, None)], str(target.origin.name)
    while name != "file":
        rule, index = via[name]
        chain.insert(0, (rule, index))
        name = str(rule.origin.name)

    def walk(depth):
        rule, index = chain[depth]
        state["rules"].append(rule)
        for i, s in enumerate(rule.expansion):
            if s.is_term:
                out.append(token(str(s.name)))
            elif i == index:
                walk(depth + 1)
            else:
                derive(str(s.name), state, out)

    walk(0)


def token(kind):
    """Лексема как пара (вид, написание)."""
    if kind in LITERAL:
        return ("lit", LITERAL[kind])
    if kind == "TERM":
        return ("TERM", None)
    if kind == "NAME":
        return ("NAME", rnd.choices(SAMPLES["NAME"], NAME_WEIGHTS)[0])
    return (kind, rnd.choice(SAMPLES[kind]))


def render(tokens, eof_term):
    """Текст из лексем: через пробел; конец инструкции — «;» или перевод строки там, где он его даёт."""
    parts, stack, prev = [], [], None
    for index, (kind, text) in enumerate(tokens):
        if kind == "TERM":
            ender = prev is not None and (
                prev[0] in nxlex.ENDER_TYPES or prev[1] in (")", "]", "}", "_") or prev[1] in nxlex.ENDER_WORDS)
            inside = bool(stack) and stack[-1] in "(["
            last = index == len(tokens) - 1
            choice = rnd.random()
            if last and eof_term and ender and choice < 0.3:
                text = ""
            elif ender and not inside and choice < 0.65:
                text = "\n"
            else:
                text = ";"
        else:
            if text in ("(", "[", "{"):
                stack.append(text)
            elif text in (")", "]", "}") and stack:
                stack.pop()
        parts.append(text)
        prev = (kind, text)
    return " ".join(parts)


def mutate(tokens):
    """Одна случайная правка в цепочке лексем."""
    tokens = list(tokens)
    kind = rnd.choice(["delete", "duplicate", "swap", "replace", "insert"])
    i = rnd.randrange(len(tokens))
    fresh = rnd.choice(VOCAB)
    fresh = token_from_vocab(fresh)
    if kind == "delete" and len(tokens) > 1:
        del tokens[i]
    elif kind == "duplicate":
        tokens.insert(i, tokens[i])
    elif kind == "swap" and i + 1 < len(tokens):
        tokens[i], tokens[i + 1] = tokens[i + 1], tokens[i]
    elif kind == "replace":
        tokens[i] = fresh
    else:
        tokens.insert(i, fresh)
    return tokens


def token_from_vocab(item):
    kind, text = item
    if kind == "lit" or kind == "TERM":
        return item
    return token(kind)


def break_line(text):
    """Перевод строки вместо одного из пробелов."""
    spaces = [i for i, c in enumerate(text) if c == " "]
    if not spaces:
        return text
    i = rnd.choice(spaces)
    return text[:i] + "\n" + text[i + 1:]


START_WEIGHTS = [("file", 55), ("expr", 15), ("stmts", 10), ("decl", 8), ("type", 6), ("pattern", 6)]
seen, lines, stats = set(), [], {"OK": 0, "ERROR": 0}


def add(case_id, start, text, rules):
    if (start, text) in seen:
        return
    seen.add((start, text))
    status, tree = ref.run(start, text)
    stats[status] += 1
    if status == "OK":
        used_rules.update(rules)
    lines.append("\t".join([case_id, start, esc(text), status] + ([esc(tree)] if tree is not None else [])))


def spoil(case_id, start, tokens, eof_term, copies):
    for k in range(copies):
        add(f"{case_id}:m{k}", start, render(mutate(tokens), eof_term), [])
    if rnd.random() < 0.5:
        add(f"{case_id}:nl", start, break_line(render(tokens, eof_term)), [])


# 1. по одному тексту на каждое раскрытие каждого правила
all_rules = [r for rules in by_origin.values() for r in rules]
number = 0
for target in all_rules:
    number += 1
    state = {"budget": rnd.choice([0, 0, 4, 12]), "rules": []}
    tokens = []
    derive_through(target, state, tokens)
    add(f"rule:{number}", "file", render(tokens, True), state["rules"])
    if tokens:
        spoil(f"rule:{number}", "file", tokens, True, 1)
targeted = len(lines)

# 2. случайные тексты
number = 0
while len(lines) < COUNT:
    number += 1
    start = rnd.choices([s for s, _ in START_WEIGHTS], [w for _, w in START_WEIGHTS])[0]
    state = {"budget": rnd.choice([3, 8, 20, 40, 80, 160]), "rules": []}
    tokens = []
    derive(start, state, tokens)
    if len(tokens) > 400 or not tokens:
        continue
    eof_term = start in ("file", "stmts")
    add(f"gen:{number}", start, render(tokens, eof_term), state["rules"])
    spoil(f"gen:{number}", start, tokens, eof_term, 2)

print(f"# случайные тексты по грамматике: зерно {SEED}; пример <TAB> стартовое правило <TAB> текст "
      f"<TAB> OK или ERROR <TAB> запись дерева")
kept = lines[:COUNT]
for line in kept:
    print(line)
print(f"текстов {len(kept)}: принято {sum(1 for x in kept if x.split(chr(9))[3] == 'OK')}, "
      f"отвергнуто {sum(1 for x in kept if x.split(chr(9))[3] == 'ERROR')}; "
      f"раскрытий правил в принятых текстах {len(used_rules)} из {len(all_rules)}", file=sys.stderr)
missing = sorted({str(r.origin.name) for r in all_rules if r not in used_rules and not str(r.origin.name).startswith("__")})
if missing:
    print("правила, у которых использованы не все раскрытия: " + " ".join(missing), file=sys.stderr)
