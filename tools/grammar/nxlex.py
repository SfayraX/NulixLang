"""Лексер черновика грамматики Nulix для проверки разборщиком Lark (LALR).

Правила, которых нет в спецификации и которые здесь приняты как предложения, помечены [П].
"""
import re
from lark import Token
from lark.lexer import Lexer

EMBEDDED = []          # выражения из "${...}" — проверяются отдельно стартовым правилом expr
EOF_TERM = True        # ставить ли конец инструкции в конце текста (для разбора целого файла)

# После этих ключевых слов конец строки завершает инструкцию (правило Go, перенесённое на Nulix) [П]
ENDER_WORDS = {"return", "break", "continue", "self", "true", "false", "any", "trust"}
ENDER_TYPES = {"NAME", "INT", "FLOAT", "STRING", "CHAR"}
# Слова, которые являются ключевыми только на своём месте
CONTEXTUAL = {"life", "end", "test", "type", "discard"}
CLOSERS = {")": "(", "]": "[", "}": "{"}

FLOAT_RE = re.compile(r"(?:0|[1-9][0-9_]*)(?:\.[0-9][0-9_]*(?:[eE][+-]?[0-9]+)?|[eE][+-]?[0-9]+)")   # [П]
INT_RE = re.compile(r"0x[0-9A-Fa-f][0-9A-Fa-f_]*|0b[01][01_]*|0o[0-7][0-7_]*|0|[1-9][0-9_]*")
ESCAPE_RE = re.compile(r"\\(?:[ntr0\\\"'$]|u\{[0-9A-Fa-f]{1,6}\})")                                   # [П]


class NxLexError(Exception):
    pass


def ident_start(ch):
    return ch == "_" or ch.isidentifier()


def ident_cont(ch):
    return ch == "_" or ("a" + ch).isidentifier()


def scan_string_body(body, line):
    """Проверяет экранирование и подстановки внутри строки."""
    i, n = 0, len(body)
    while i < n:
        c = body[i]
        if c == "\\":
            m = ESCAPE_RE.match(body, i)
            if not m:
                raise NxLexError(f"строка {line}: неизвестное экранирование {body[i:i+2]!r}")
            i = m.end()
        elif c == "$" and i + 1 < n and body[i + 1] == "{":
            depth, j = 1, i + 2
            while j < n and depth:
                depth += {"{": 1, "}": -1}.get(body[j], 0)
                j += 1
            if depth:
                raise NxLexError(f"строка {line}: не закрыта подстановка ${{…}}")
            EMBEDDED.append((line, body[i + 2:j - 1]))
            i = j
        elif c == "$" and i + 1 < n and ident_start(body[i + 1]):
            i += 2
            while i < n and ident_cont(body[i]):
                i += 1
        else:
            i += 1


class NxLexer(Lexer):
    def __init__(self, lexer_conf):
        self.lit = {}
        for t in lexer_conf.terminals:
            p = t.pattern
            if p is not None and type(p).__name__ == "PatternStr":
                self.lit[p.value] = t.name
        self.words = {v for v in self.lit if v[0].isalpha()}
        self.ops = sorted((v for v in self.lit if not v[0].isalpha() and v[0] not in "@_"),
                          key=len, reverse=True)

    def lex(self, text):
        out, stack = [], []
        i, n, line = 0, len(text), 1

        def emit(kind, value):
            out.append(Token(kind, value, line=line))

        def ends_statement():
            if not out:
                return False
            t = out[-1]
            if t.type == "TERM":
                return False
            if len(out) > 1 and out[-2].type == self.lit["end"]:
                return True                      # лексема после end — это метка конца блока
            if t.type in ENDER_TYPES or t.value in CLOSERS or t.value == "_":
                return True
            return t.value in self.words and t.value in ENDER_WORDS

        def skip_blank(k):
            while k < n:
                if text[k] in " \t\r\n":
                    k += 1
                elif text.startswith("//", k):
                    while k < n and text[k] != "\n":
                        k += 1
                elif text.startswith("/*", k):
                    depth, k = 1, k + 2
                    while k < n and depth:
                        if text.startswith("/*", k):
                            depth, k = depth + 1, k + 2
                        elif text.startswith("*/", k):
                            depth, k = depth - 1, k + 2
                        else:
                            k += 1
                else:
                    break
            return k

        def contextual_keyword(word, prev, k):
            """Ключевое ли здесь слово из CONTEXTUAL; k — позиция первого знака после слова."""
            after = text[k] if k < n else ""
            j = k
            while j < n and ident_cont(text[j]):
                j += 1
            nxt = text[k:j]
            if word == "life":      # после "[" или "," и перед именем или "("
                return prev is not None and prev.value in ("[", ",") and (after == "(" or bool(nxt))
            if word == "end":       # только сразу после "}"
                return prev is not None and prev.type == self.lit["}"]
            if word == "test":      # только перед строкой
                return after == '"'
            if word == "type":      # только перед именем
                return bool(nxt) and ident_start(after) and (nxt not in self.words or nxt in CONTEXTUAL)
            return nxt == "self"    # discard — только перед self

        def newline(pos):
            # внутри круглых и квадратных скобок перевод строки ничего не завершает
            if stack and stack[-1] in "([":
                return
            k = skip_blank(pos)
            if text.startswith(".", k) and not text.startswith("..", k):
                return                           # строка, начинающаяся с точки, продолжает предыдущую
            if ends_statement():
                emit("TERM", "\n")

        while i < n:
            c = text[i]
            if c == "\n":
                newline(i + 1)
                line += 1
                i += 1
            elif c in " \t\r":
                i += 1
            elif text.startswith("//", i):
                while i < n and text[i] != "\n":
                    i += 1
            elif text.startswith("/*", i):
                depth, i = 1, i + 2
                while i < n and depth:
                    if text.startswith("/*", i):
                        depth, i = depth + 1, i + 2
                    elif text.startswith("*/", i):
                        depth, i = depth - 1, i + 2
                    else:
                        line += text[i] == "\n"
                        i += 1
                if depth:
                    raise NxLexError(f"строка {line}: не закрыт комментарий /* */")
            elif c == ";":
                if out and out[-1].type != "TERM":
                    emit("TERM", ";")
                i += 1
            elif text.startswith('"""', i):
                j = text.find('"""', i + 3)
                if j < 0:
                    raise NxLexError(f"строка {line}: не закрыта многострочная строка")
                scan_string_body(text[i + 3:j], line)
                emit("STRING", text[i:j + 3])
                line += text.count("\n", i, j)
                i = j + 3
            elif c == '"':
                j = i + 1
                while j < n and text[j] != '"':
                    if text[j] == "\n":
                        raise NxLexError(f"строка {line}: перевод строки внутри \"…\"")
                    j += 2 if text[j] == "\\" else 1
                if j >= n:
                    raise NxLexError(f"строка {line}: не закрыта строка")
                scan_string_body(text[i + 1:j], line)
                emit("STRING", text[i:j + 1])
                i = j + 1
            elif c == "'":
                m = ESCAPE_RE.match(text, i + 1)
                j = m.end() if m else i + 2
                if j >= n or text[j] != "'" or text[i + 1] in "'\n":
                    raise NxLexError(f"строка {line}: неверный символьный литерал")
                emit("CHAR", text[i:j + 1])
                i = j + 1
            elif c.isdigit():
                if c == "0" and i + 1 < n and text[i + 1].isdigit():
                    raise NxLexError(f"строка {line}: ведущий ноль в числе")
                m = FLOAT_RE.match(text, i)
                kind = "FLOAT"
                if not m:
                    m, kind = INT_RE.match(text, i), "INT"
                i = m.end()
                if i < n and ident_cont(text[i]):
                    raise NxLexError(f"строка {line}: буква сразу после числа {m.group()!r}")
                emit(kind, m.group())
            elif c == "@":
                j = i + 1
                while j < n and ident_cont(text[j]):
                    j += 1
                word = text[i:j]
                if j == i + 1:
                    raise NxLexError(f"строка {line}: после @ нужно имя пометки")
                emit(self.lit.get(word, "ATTR"), word)
                i = j
            elif ident_start(c):
                j = i + 1
                while j < n and ident_cont(text[j]):
                    j += 1
                word = text[i:j]
                i = j
                prev = out[-1] if out else None
                k = i
                while k < n and text[k] in " \t":
                    k += 1
                if word == "_":
                    emit(self.lit["_"], word)
                elif word not in self.words:
                    emit("NAME", word)
                elif prev is not None and prev.type in (self.lit["."], self.lit["func"]):
                    emit("NAME", word)           # после точки и после func любое слово — имя
                elif word in CONTEXTUAL:
                    emit(self.lit[word] if contextual_keyword(word, prev, k) else "NAME", word)
                else:
                    emit(self.lit[word], word)
            else:
                for op in self.ops:
                    if text.startswith(op, i):
                        break
                else:
                    raise NxLexError(f"строка {line}: неизвестный знак {c!r}")
                if op in "([{":
                    stack.append(op)
                elif op in CLOSERS:
                    if not stack or stack[-1] != CLOSERS[op]:
                        raise NxLexError(f"строка {line}: лишняя или непарная скобка {op!r}")
                    stack.pop()
                emit(self.lit[op], op)
                i += len(op)
        if stack:
            raise NxLexError(f"конец текста: не закрыта скобка {stack[-1]!r}")
        if EOF_TERM and ends_statement():
            emit("TERM", "")
        return iter(out)
