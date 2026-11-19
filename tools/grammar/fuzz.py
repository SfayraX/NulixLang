"""Тексты, выведенные из грамматики, и их порча — для сверки разборщика на Rust с проверочным.

Тексты выводятся из правил grammar/nulix.lark: сначала по одному тексту на каждое раскрытие каждого
правила, потом случайные. Каждый текст ещё и портится одной правкой: лишняя, пропущенная или
переставленная лексема либо целая часть (например, второе `uses` или `without` раньше `uses`),
перевод строки не на месте, лишний или пропущенный знак. Все тексты разбирает проверочный разборщик, и его исход с записью
дерева попадает в файл. Разборщик на Rust обязан выдать то же самое на каждом тексте.

Запуск: python3 tools/grammar/fuzz.py grammar <сколько текстов> <зерно> [--max-tokens N] [--exhaustive]
  --max-tokens N   не брать тексты длиннее N лексем (по умолчанию 400)
  --exhaustive     для текстов «по раскрытиям» — ещё все правки по очереди: пропуск и повтор каждой
                   лексемы и каждой части, перестановка соседних частей, замена слова на похожее
"""
import random
import sys

from canon import Reference, esc
import nxlex

sys.setrecursionlimit(20000)
G, COUNT, SEED = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
MAX_TOKENS = int(sys.argv[sys.argv.index("--max-tokens") + 1]) if "--max-tokens" in sys.argv else 400
EXHAUSTIVE = "--exhaustive" in sys.argv
rnd = random.Random(SEED)
ref = Reference(G)

# ── правила грамматики после раскрытия повторов и необязательных частей ──
LITERAL = {t.name: t.pattern.value for t in ref.parser.terminals if t.pattern is not None}
by_origin = {}
for rule in ref.parser.rules:
    by_origin.setdefault(str(rule.origin.name), []).append(rule)
all_rules = [r for rules in by_origin.values() for r in rules]

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
# слова и знаки, которые легко перепутать: замена внутри группы даёт самые коварные ошибки
GROUPS = [["and", "or"], ["uses", "without", "limit"], ["==", "!=", "<", "<=", ">", ">=", "is"],
          ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>", "..<", "..="], ["mut", "take"], ["ref", "ptr"],
          ["let", "var", "const"], ["for", "while", "loop"], ["if", "else", "match", "case"],
          ["=", "+=", "<<=", ":", "->"], [",", ";", "."], ["try", "await", "not", "~"], ["any", "self", "trust"],
          ["struct", "enum", "interface", "impl", "type", "test", "extern", "func"], ["end", "catch", "pub"]]
SIMILAR = {}
for group in GROUPS:
    for word in group:
        SIMILAR[word] = [w for w in group if w != word and (w in LITERAL.values() or w == ";")]
VOCAB = [("lit", v) for v in LITERAL.values()] + [(k, None) for k in SAMPLES] + [("TERM", None)]
used_rules = set()


def token(kind):
    """Лексема как пара (вид, написание)."""
    if kind in LITERAL:
        return ("lit", LITERAL[kind])
    if kind == "TERM":
        return ("TERM", None)
    if kind == "NAME":
        return ("NAME", rnd.choices(SAMPLES["NAME"], NAME_WEIGHTS)[0])
    return (kind, rnd.choice(SAMPLES[kind]))


def choose(name, state):
    """Раскрытие правила: пока есть запас — любое, чаще ещё не встречавшееся; потом самое короткое."""
    rules = by_origin[name]
    fresh = [r for r in rules if r not in used_rules]
    if state["budget"] > 0 and fresh and rnd.random() < 0.5:
        rule = rnd.choice(fresh)
    elif state["budget"] > 0:
        rule = rnd.choice(rules)
    else:
        low = min(rule_cost(r) for r in rules)
        rule = rnd.choice([r for r in rules if rule_cost(r) == low])
    state["budget"] -= 1
    return rule


def expand(rule, state, out, chain=None, depth=0):
    """Применяет раскрытие и запоминает, какие лексемы дала каждая часть.

    chain — путь от файла к раскрытию, которое обязано встретиться: пары (раскрытие, место в нём).
    """
    state["rules"].append(rule)
    start, kids = len(out), []
    forced = chain[depth][1] if chain else None
    for index, s in enumerate(rule.expansion):
        if s.is_term:
            out.append(token(str(s.name)))
            continue
        kid_start = len(out)
        if chain and index == forced:
            expand(chain[depth + 1][0], state, out, chain, depth + 1)
        else:
            expand(choose(str(s.name), state), state, out)
        kids.append((kid_start, len(out)))
    state["spans"].append((start, len(out)))
    for a, b in zip(kids, kids[1:]):
        if a[0] < a[1] == b[0] < b[1]:
            state["pairs"].append((a, b))


def path_to(target):
    chain, name = [(target, None)], str(target.origin.name)
    while name != "file":
        rule, index = via[name]
        chain.insert(0, (rule, index))
        name = str(rule.origin.name)
    return chain


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
        elif text in ("(", "[", "{"):
            stack.append(text)
        elif text in (")", "]", "}") and stack:
            stack.pop()
        parts.append(text)
        prev = (kind, text)
    return " ".join(parts)


def spoil_token(tokens):
    """Одна случайная правка одной лексемы."""
    tokens = list(tokens)
    kind = rnd.choice(["delete", "duplicate", "swap", "replace", "insert"])
    i = rnd.randrange(len(tokens))
    fresh = rnd.choice(VOCAB)
    if fresh[0] not in ("lit", "TERM"):
        fresh = token(fresh[0])
    if kind == "delete" and len(tokens) > 1:
        del tokens[i]
    elif kind == "duplicate":
        tokens.insert(i, tokens[i])
    elif kind == "swap" and i + 1 < len(tokens):
        tokens[i], tokens[i + 1] = tokens[i + 1], tokens[i]
    elif kind == "replace" and tokens[i][1] in SIMILAR and rnd.random() < 0.7:
        tokens[i] = similar(rnd.choice(SIMILAR[tokens[i][1]]))
    elif kind == "replace":
        tokens[i] = fresh
    else:
        tokens.insert(i, fresh)
    return tokens


def similar(word):
    return ("TERM", None) if word == ";" else ("lit", word)


def spoil_part(tokens, state):
    """Одна правка целой части текста: пропуск, повтор или перестановка двух соседних частей."""
    kind = rnd.choice(["delete", "duplicate", "swap"])
    if kind == "swap" and state["pairs"]:
        (a0, a1), (b0, b1) = rnd.choice(state["pairs"])
        return tokens[:a0] + tokens[b0:b1] + tokens[a0:a1] + tokens[b1:]
    spans = [s for s in state["spans"] if 0 < s[1] - s[0] < len(tokens)]
    short = [s for s in spans if s[1] - s[0] <= 6]
    if not spans:
        return None
    s0, s1 = rnd.choice(short or spans)
    if kind == "delete":
        return tokens[:s0] + tokens[s1:]
    return tokens[:s1] + tokens[s0:s1] + tokens[s1:]


def break_line(text):
    """Перевод строки вместо одного из пробелов."""
    spaces = [i for i, c in enumerate(text) if c == " "]
    if not spaces:
        return text
    i = rnd.choice(spaces)
    return text[:i] + "\n" + text[i + 1:]


CHARS = list("\"'\\${}()[].;:,@#_/*-+<>=!~^&|% \n\t") + ["0", "7", "x", "e", "a", "я", "/*", "*/", "//", '"""']


def spoil_char(text):
    """Одна правка на уровне знаков: пропущенный, лишний или заменённый знак."""
    kind = rnd.choice(["delete", "insert", "replace"])
    if not text or kind == "insert":
        i = rnd.randrange(len(text) + 1)
        return text[:i] + rnd.choice(CHARS) + text[i:]
    i = rnd.randrange(len(text))
    return text[:i] + (rnd.choice(CHARS) if kind == "replace" else "") + text[i + 1:]


seen, lines = set(), []


def add(case_id, start, text, rules):
    if (start, text) in seen:
        return
    seen.add((start, text))
    status, tree = ref.run(start, text)
    if status == "OK":
        used_rules.update(rules)
    lines.append("\t".join([case_id, start, esc(text), status] + ([esc(tree)] if tree is not None else [])))


def spoil(case_id, start, tokens, state, copies):
    eof_term = start in ("file", "stmts")
    for k in range(copies):
        add(f"{case_id}:t{k}", start, render(spoil_token(tokens), eof_term), [])
        part = spoil_part(tokens, state)
        if part:
            add(f"{case_id}:p{k}", start, render(part, eof_term), [])
    if rnd.random() < 0.4:
        add(f"{case_id}:nl", start, break_line(render(tokens, eof_term)), [])
    if rnd.random() < 0.5:
        add(f"{case_id}:ch", start, spoil_char(render(tokens, eof_term)), [])


def new_state(budgets):
    return {"budget": rnd.choice(budgets), "rules": [], "spans": [], "pairs": []}


# 1. по одному тексту на каждое раскрытие каждого правила
for number, target in enumerate(all_rules, 1):
    state, tokens = new_state([0, 0, 4, 12]), []
    chain = path_to(target)
    expand(chain[0][0], state, tokens, chain)
    add(f"rule:{number}", "file", render(tokens, True), state["rules"])
    if not tokens:
        continue
    spoil(f"rule:{number}", "file", tokens, state, 1)
    if EXHAUSTIVE:
        for i in range(len(tokens)):
            add(f"rule:{number}:del{i}", "file", render(tokens[:i] + tokens[i + 1:], True), [])
            add(f"rule:{number}:dup{i}", "file", render(tokens[:i + 1] + tokens[i:], True), [])
            for k, word in enumerate(SIMILAR.get(tokens[i][1], [])):
                add(f"rule:{number}:sim{i}.{k}", "file", render(tokens[:i] + [similar(word)] + tokens[i + 1:], True), [])
        for k, (s0, s1) in enumerate(sorted(set(state["spans"]))):
            if 1 < s1 - s0 < len(tokens):
                add(f"rule:{number}:pdel{k}", "file", render(tokens[:s0] + tokens[s1:], True), [])
                add(f"rule:{number}:pdup{k}", "file", render(tokens[:s1] + tokens[s0:s1] + tokens[s1:], True), [])
        for k, ((a0, a1), (b0, b1)) in enumerate(state["pairs"]):
            add(f"rule:{number}:swap{k}", "file",
                render(tokens[:a0] + tokens[b0:b1] + tokens[a0:a1] + tokens[b1:], True), [])

# 2. случайные тексты с разных стартовых правил
START_WEIGHTS = [("file", 50), ("expr", 18), ("stmts", 10), ("decl", 8), ("type", 7), ("pattern", 7)]
number = 0
while len(lines) < COUNT:
    number += 1
    start = rnd.choices([s for s, _ in START_WEIGHTS], [w for _, w in START_WEIGHTS])[0]
    state, tokens = new_state([3, 8, 20, 40, 80, 160]), []
    expand(choose(start, state), state, tokens)
    if not tokens or len(tokens) > MAX_TOKENS:
        continue
    add(f"gen:{number}", start, render(tokens, start in ("file", "stmts")), state["rules"])
    spoil(f"gen:{number}", start, tokens, state, 2)

kept = lines if EXHAUSTIVE else lines[:COUNT]
print(f"# тексты, выведенные из грамматики: зерно {SEED}; раскрытий правил в принятых текстах "
      f"{len(used_rules)} из {len(all_rules)}; пример <TAB> стартовое правило <TAB> текст "
      f"<TAB> OK или ERROR <TAB> запись дерева")
for line in kept:
    print(line)
accepted = sum(1 for x in kept if x.split("\t")[3] == "OK")
clean = [x for x in kept if x.split("\t")[0].count(":") == 1]
clean_bad = sum(1 for x in clean if x.split("\t")[3] == "ERROR")
print(f"текстов {len(kept)}: принято {accepted}, отвергнуто {len(kept) - accepted}; "
      f"неиспорченных {len(clean)}, из них отвергнуто {clean_bad}; "
      f"раскрытий правил в принятых текстах {len(used_rules)} из {len(all_rules)}", file=sys.stderr)
missing = sorted({str(r.origin.name) for r in all_rules if r not in used_rules})
if missing:
    print("правила, у которых использованы не все раскрытия: " + " ".join(missing), file=sys.stderr)
