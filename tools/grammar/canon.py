"""Каноническая запись дерева разбора и проверочный разборщик.

Запись — скобочная: (вид часть часть …). Её выдают две независимые реализации: эта, по дереву
Lark, и разборщик на Rust (compiler/src/dump.rs). Сверка записей на общих примерах показывает,
что разборщик на Rust разбирает текст так же, как грамматика grammar/nulix.lark.

В запись не входят скобки вокруг типа, завершающие запятые и концы инструкций.
"""
import os

from lark import Lark, Token, Tree
from lark.exceptions import LarkError

import nxlex

NAMED = {"NAME", "INT", "FLOAT", "STRING", "CHAR", "ATTR", "TERM"}


def node(head, *parts):
    return "(" + " ".join([head] + [p for p in parts if p is not None]) + ")"


def is_sym(x, *texts):
    """Ключевое слово, пометка или знак с одним из данных написаний."""
    return isinstance(x, Token) and x.type not in NAMED and x.value in texts


def named(x, kind):
    return isinstance(x, Token) and x.type == kind


def sub(t, name):
    """Первое поддерево с данным правилом или None."""
    for c in t.children:
        if isinstance(c, Tree) and c.data == name:
            return c
    return None


def has_end(t):
    return "end" if any(is_sym(c, "end") for c in t.children) else None


def string_subs(raw):
    """Подстановки внутри строкового литерала: ("name", имя) и ("expr", текст выражения)."""
    body = raw[3:-3] if raw.startswith('"""') else raw[1:-1]
    subs, i, n = [], 0, len(body)
    while i < n:
        c = body[i]
        if c == "\\":
            i = nxlex.ESCAPE_RE.match(body, i).end()      # запись экранирования уже проверил лексер
        elif c == "$" and i + 1 < n and body[i + 1] == "{":
            depth, j = 1, i + 2
            while j < n and depth:
                depth += {"{": 1, "}": -1}.get(body[j], 0)
                j += 1
            subs.append(("expr", body[i + 2:j - 1]))
            i = j
        elif c == "$" and i + 1 < n and nxlex.ident_start(body[i + 1]):
            j = i + 2
            while j < n and nxlex.ident_cont(body[j]):
                j += 1
            subs.append(("name", body[i + 1:j]))
            i = j
        else:
            i += 1
    return subs


class Canon:
    def __init__(self, parse_expr):
        self.parse_expr = parse_expr        # текст → дерево Lark по правилу expr

    def go(self, t):
        return getattr(self, "r_" + t.data)(t)

    def many(self, t):
        """Записи всех поддеревьев подряд; лексемы пропускаются."""
        return [self.go(c) for c in t.children if isinstance(c, Tree)]

    def opt(self, t, name):
        x = sub(t, name)
        return self.go(x) if x is not None else None

    def names(self, t):
        """Имена через точку: правила вида x: NAME | x "." NAME."""
        out = []
        for c in t.children:
            if isinstance(c, Tree):
                out += self.names(c)
            elif c.type == "NAME":
                out.append(c.value)
        return out

    def string(self, tok):
        parts = []
        for kind, text in string_subs(tok.value):
            if kind == "name":
                parts.append(node("sub-name", text))
            else:
                parts.append(node("sub", self.go(self.parse_expr(text))))
        return node("str", tok.value, *parts)

    def mode(self, t):
        m = sub(t, "mode")
        return node("mode", m.children[0].value) if m is not None else None

    def tail(self, t):
        """Результат и требования после параметров: [(-> тип)] [(reqs …)]."""
        if t is None:
            return []
        out = []
        for c in t.children:
            if isinstance(c, Tree):
                out.append(self.go(c) if c.data == "reqs" else node("->", self.go(c)))
        return out

    def req_items(self, t):
        return self.many(t)

    def with_attrs(self, head, t):
        parts = []
        for c in t.children:
            if isinstance(c, Tree) and c.data == "attrs":
                parts += self.many(c)
            elif isinstance(c, Tree):
                parts.append(self.go(c))
            else:
                parts.append("pub")
        return node(head, *parts)

    # ───────── файл ─────────
    def r_file(self, t):
        return node("file", *self.many(t))

    def r_file_header(self, t):
        return node("header", *self.many(t))

    def r_header_arg(self, t):
        c = t.children
        return node("harg", c[0].value) if len(c) == 1 else node("harg-target", c[2].value)

    def r_import_decl(self, t):
        c = t.children
        path = node("path", *self.names(c[1]))
        if len(c) == 2:
            return node("import", path)
        if is_sym(c[2], "as"):
            return node("import", path, node("as", c[3].value))
        return node("import", path, node("group", *[self.go(x) for x in c[2:] if isinstance(x, Tree)]))

    def r_import_name(self, t):
        c = t.children
        return node("item", c[0].value, node("as", c[2].value) if len(c) > 1 else None)

    def r_top_item(self, t):
        return self.go(t.children[0])

    # ───────── объявления ─────────
    def r_decl(self, t):
        return self.with_attrs("decl", t)

    def r_attr(self, t):
        c = t.children
        if isinstance(c[0], Tree):
            return self.go(c[0])
        if len(c) == 1:
            return node("attr", c[0].value)
        a = sub(t, "args")
        return node("attr", c[0].value, node("args", *(self.many(a) if a is not None else [])))

    def r_level(self, t):
        return node("level", t.children[0].value)

    def r_strictness(self, t):
        return node("strictness", t.children[0].value)

    def r_decl_core(self, t):
        return self.go(t.children[0])

    def r_binding(self, t):
        c = t.children
        ty = sub(t, "type")
        return node(c[0].value, self.go(c[1]), node(":", self.go(ty)) if ty is not None else None, self.go(c[-1]))

    def r_bind_target(self, t):
        c = t.children[0]
        return self.go(c) if isinstance(c, Tree) else node("bind", c.value)

    def r_func_decl(self, t):
        return node("func", *self.many(t))

    def r_extern_func(self, t):
        return node("extern-func", self.string(t.children[1]), *self.many(t))

    def r_func_sig(self, t):
        p = sub(t, "params")
        return node("sig", t.children[1].value, self.opt(t, "generics"),
                    node("params", *(self.many(p) if p is not None else [])), *self.tail(sub(t, "sig_tail")))

    def r_sig_tail(self, t):                 # только как стартовое правило
        return node("sig-tail", *self.tail(t))

    def r_end_name(self, t):
        return node("end", t.children[1].value)

    def r_generics(self, t):
        return node("generics", *self.many(t))

    def r_generic_param(self, t):
        c = t.children
        if is_sym(c[0], "uses"):
            w = sub(t, "req_set")
            return node("rparam", c[1].value, node("without", *self.req_items(w)) if w is not None else None)
        if is_sym(c[0], "life"):
            return node("lparam", c[1].value)
        return node("tparam", c[0].value, self.opt(t, "bound"))

    def r_bound(self, t):
        return node("bound", *self.many(t))

    def r_param(self, t):
        c = t.children
        if is_sym(c[-1], "self"):
            return node("self-param", self.mode(t))
        ty = sub(t, "type")
        name = [x for x in c if named(x, "NAME")][0].value
        return node("param", self.mode(t), name, node(":", self.go(ty)) if ty is not None else None)

    def r_reqs(self, t):
        parts = []
        for c in t.children:
            if c.data == "uses_clause":
                parts.append(node("uses", *self.req_items(c.children[1])))
            elif c.data == "without_clause":
                parts.append(node("without", *self.req_items(c.children[1])))
            else:
                parts.append(node("limit", *self.many(c.children[1])))
        return node("reqs", *parts)

    def r_req_item(self, t):
        c = t.children[0]
        return node("req", *self.names(c)) if isinstance(c, Tree) else node("req-trust")

    def r_limit_item(self, t):
        c = t.children
        return node("limit-le" if c[1].value == "<=" else "limit-ge", c[0].value, self.go(c[2]))

    def r_struct_decl(self, t):
        f = sub(t, "fields")
        return node("struct", t.children[1].value, self.opt(t, "generics"),
                    *(self.many(f) if f is not None else []), self.opt(t, "end_name"))

    def r_field(self, t):
        c = t.children
        a = sub(t, "attrs")
        e = sub(t, "expr")
        name = [x for x in c if named(x, "NAME")][0].value
        return node("field", name, *(self.many(a) if a is not None else []),
                    "pub" if any(is_sym(x, "pub") for x in c) else None,
                    self.go(sub(t, "type")), node("default", self.go(e)) if e is not None else None)

    def r_enum_decl(self, t):
        v = sub(t, "variants")
        return node("enum", t.children[1].value, self.opt(t, "generics"),
                    *(self.many(v) if v is not None else []), self.opt(t, "end_name"))

    def r_variant(self, t):
        c = t.children
        name = [x for x in c if named(x, "NAME")][0].value
        return node("variant", name, "indirect" if is_sym(c[0], "indirect") else None, self.opt(t, "variant_fields"))

    def r_variant_fields(self, t):
        ty = sub(t, "type")
        return node("positional", self.go(ty)) if ty is not None else node("named", *self.many(t))

    def r_named_field(self, t):
        return node("nf", t.children[0].value, self.go(t.children[2]))

    def r_interface_decl(self, t):
        m = sub(t, "members")
        return node("interface", t.children[1].value, self.opt(t, "generics"), self.opt(t, "bound"),
                    *(self.many(m) if m is not None else []), self.opt(t, "end_name"))

    def r_impl_decl(self, t):
        paths = [x for x in t.children if isinstance(x, Tree) and x.data == "path_type"]
        m = sub(t, "members")
        return node("impl", self.opt(t, "generics"), self.go(paths[0]),
                    node("for", self.go(paths[1])) if len(paths) > 1 else None,
                    *(self.many(m) if m is not None else []), self.opt(t, "end_impl"))

    def r_end_impl(self, t):
        ns = [node("tname", *self.names(x)) for x in t.children if isinstance(x, Tree)]
        return node("end-impl", ns[0], node("for", ns[1]) if len(ns) > 1 else None)

    def r_member(self, t):
        return self.with_attrs("member", t)

    def r_type_decl(self, t):
        return node("type-alias", t.children[1].value, self.opt(t, "generics"), self.go(sub(t, "type")))

    def r_test_decl(self, t):
        return node("test", self.string(t.children[1]), self.go(sub(t, "block")), has_end(t))

    def r_extern_block(self, t):
        c = t.children
        m = sub(t, "members")
        return node("extern-block", self.string(c[1]), self.string(c[3]),
                    *(self.many(m) if m is not None else []), has_end(t))

    # ───────── типы ─────────
    def r_type(self, t):
        return self.go(t.children[0])

    def r_closed_type(self, t):
        c = t.children
        if isinstance(c[0], Tree):
            return self.go(c[0])
        types = self.many(t)
        if not types:
            return node("t-unit")
        return types[0] if len(types) == 1 else node("t-tuple", *types)

    def r_simple_type(self, t):
        c = t.children
        if isinstance(c[0], Tree):
            return self.go(c[0])
        if is_sym(c[0], "any"):
            return node("t-any", self.go(c[1]) if len(c) > 1 else None)
        mutable = is_sym(c[0], "mut")
        word = c[1].value if mutable else c[0].value          # ref или ptr
        return node(("t-mut" if mutable else "t-") + word, self.opt(t, "origin"), self.go(c[-1]))

    r_open_type = r_simple_type

    def r_origin(self, t):
        x = t.children[1]
        return node("origin-self") if is_sym(x, "self") else node("origin", x.value)

    def r_path_type(self, t):
        ga = sub(t, "generic_args")
        return node("t-path", *self.names(t.children[0]), self.go(ga) if ga is not None else None)

    def r_generic_args(self, t):
        return node("targs", *self.many(t))

    def r_generic_arg(self, t):
        c = t.children
        if isinstance(c[0], Tree):
            return self.go(c[0])
        if named(c[0], "INT"):
            return node("int", c[0].value)
        if is_sym(c[0], "uses"):
            return node("uses", *self.req_items(c[1]))
        return self.go(c[1])

    def r_life_src(self, t):
        c = t.children
        if is_sym(c[0], "self"):
            return node("life-self")
        return node("life", *[x.value for x in c if named(x, "NAME")])

    def r_fn_type(self, t):
        p = sub(t, "fn_params")
        return node("t-func", self.opt(t, "fn_kind"), node("params", *(self.many(p) if p is not None else [])),
                    *self.tail(sub(t, "fn_tail")))

    def r_fn_kind(self, t):
        c = t.children
        if is_sym(c[0], "extern"):
            return node("kind", "extern", self.string(c[1]))
        return node("kind", c[0].value)

    def r_fn_param(self, t):
        c = t.children
        ty = self.go(sub(t, "type"))
        if len(c) == 1:
            return node("fparam", ty)
        name = [x for x in c if named(x, "NAME") or is_sym(x, "_")][0].value
        return node("fparam", self.mode(t), name, ty)

    # ───────── инструкции ─────────
    def r_block(self, t):
        s = sub(t, "stmts")
        return node("block", *(self.many(s) if s is not None else []))

    def r_stmts(self, t):                    # только как стартовое правило
        return node("stmts", *self.many(t))

    def r_stmt(self, t):
        return self.go(t.children[0])

    def r_plain_stmt(self, t):
        c = t.children
        if isinstance(c[0], Tree):
            return self.go(c[0])
        word = c[0].value
        if word == "return":
            return node("return", self.go(c[1]) if len(c) > 1 else None)
        if word in ("break", "continue"):
            return node(word, c[1].value if len(c) > 1 else None)
        if word == "assert":
            return node("assert", *self.many(t))
        return node("discard-self")

    def r_assign(self, t):
        c = t.children
        return node("assign", c[1].children[0].value, self.go(c[0]), self.go(c[2]))

    def r_for_stmt(self, t):
        return node("for", self.opt(t, "label"), self.go(sub(t, "for_target")), self.mode(t),
                    self.go(sub(t, "expr")), self.go(sub(t, "block")), self.opt(t, "end_loop"))

    def r_while_stmt(self, t):
        return node("while", self.opt(t, "label"), self.go(sub(t, "expr")), self.go(sub(t, "block")),
                    self.opt(t, "end_loop"))

    def r_loop_stmt(self, t):
        return node("loop", self.opt(t, "label"), self.go(sub(t, "block")), self.opt(t, "end_loop"))

    def r_label(self, t):
        return node("label", t.children[0].value)

    def r_for_target(self, t):
        c = t.children[0]
        if isinstance(c, Tree):
            return self.go(c)
        return node("p-wild") if is_sym(c, "_") else node("bind", c.value)

    def r_end_loop(self, t):
        x = t.children[1]
        return node("end", x.value) if named(x, "NAME") else node("end-kw", x.value)

    # ───────── выражения ─────────
    def r_expr(self, t):
        return self.go(t.children[0])

    def chain_left(self, head, t):
        items = self.many(t)
        acc = items[0]
        for x in items[1:]:
            acc = node(head, acc, x)
        return acc

    def r_or_expr(self, t):
        return self.chain_left("or", t)

    def r_and_expr(self, t):
        return self.chain_left("and", t)

    def r_not_expr(self, t):
        c = t.children
        return self.go(c[0]) if len(c) == 1 else node("not", self.go(c[1]))

    def r_comparison(self, t):
        c = t.children
        if len(c) == 1:
            return self.go(c[0])
        if is_sym(c[1], "is"):
            return node("is", self.go(c[0]), self.go(c[2]))
        return node(c[1].children[0].value, self.go(c[0]), self.go(c[2]))

    def r_operand(self, t):
        return self.go(t.children[0])

    def binary(self, t):
        c = t.children
        return self.go(c[0]) if len(c) == 1 else node(c[1].value, self.go(c[0]), self.go(c[2]))

    r_range = r_arith = r_term = r_bit_and = r_bit_or = r_bit_xor = r_shift = binary

    def r_unary(self, t):
        c = t.children
        if len(c) == 1:
            return self.go(c[0])
        name = [x for x in c if named(x, "NAME")]
        return node("catch", self.go(c[0]), node("err", name[0].value) if name else None,
                    self.go(sub(t, "block")), has_end(t))

    def r_prefix(self, t):
        c = t.children
        if len(c) == 1:
            return self.go(c[0])
        return node({"-": "neg", "~": "bitnot", "try": "try", "await": "await"}[c[0].value], self.go(c[1]))

    def r_postfix(self, t):
        c = t.children
        if len(c) == 1:
            return self.go(c[0])
        base = self.go(c[0])
        if is_sym(c[1], "."):
            return node(".", base, c[2].value)
        if is_sym(c[1], "("):
            a = sub(t, "args")
            return node("call", base, *(self.many(a) if a is not None else []))
        return node("index", base, *self.many(sub(t, "index_args")))

    def r_primary(self, t):
        c = t.children
        if isinstance(c[0], Tree):
            return self.go(c[0])
        if named(c[0], "NAME"):
            return node("name", c[0].value)
        if is_sym(c[0], "self"):
            return node("self")
        items = self.many(t)
        if is_sym(c[0], "["):
            return node("list", *items)
        if not items:
            return node("unit")
        return node("paren", items[0]) if len(items) == 1 else node("tuple", *items)

    def r_literal(self, t):
        x = t.children[0]
        if named(x, "STRING"):
            return self.string(x)
        for kind, head in (("INT", "int"), ("FLOAT", "float"), ("CHAR", "char")):
            if named(x, kind):
                return node(head, x.value)
        return node(x.value)                 # true или false

    def r_arg(self, t):
        c = t.children
        if len(c) == 1:
            return self.go(c[0])
        if is_sym(c[0], "mut"):
            return node("mut-arg", self.go(c[1]))
        return node("named-arg", c[0].value, self.go(c[2]))

    def r_index_arg(self, t):
        c = t.children
        if isinstance(c[0], Tree):
            return self.go(c[0])
        if is_sym(c[0], "any"):
            return node("t-any", self.go(c[1]) if len(c) > 1 else None)
        return node("uses", *self.req_items(c[1]))

    def r_if_expr(self, t):
        return self.if_chain(t.children[0], has_end(t))

    def if_chain(self, t, end=None):
        c = t.children                       # if условие блок [else блок | else if …]
        other = None
        if len(c) > 3:
            other = node("else", self.if_chain(c[4]) if c[4].data == "if_chain" else self.go(c[4]))
        return node("if", self.go(c[1]), self.go(c[2]), other, end)

    def r_match_expr(self, t):
        items = [self.go(x) for x in t.children if isinstance(x, Tree) and x.data == "match_item"]
        return node("match", self.mode(t), self.go(sub(t, "expr")), *items, has_end(t))

    def r_match_item(self, t):
        c = t.children
        if is_sym(c[0], "else"):
            return node("case-else", self.go(c[1]), has_end(t))
        pats = [self.go(x) for x in c if isinstance(x, Tree) and x.data == "pattern"]
        g = sub(t, "expr")
        return node("case", node("pats", *pats), node("guard", self.go(g)) if g is not None else None,
                    self.go(sub(t, "block")), has_end(t))

    def r_closure(self, t):
        p = sub(t, "params")
        return node("closure", node("params", *(self.many(p) if p is not None else [])),
                    *self.tail(sub(t, "sig_tail")), self.go(sub(t, "block")), has_end(t))

    def r_trust_expr(self, t):
        return node("trust", self.string(t.children[1]), self.go(sub(t, "block")), has_end(t))

    def r_provide_expr(self, t):
        return node("provide", *self.many(sub(t, "provide_set")), self.go(sub(t, "block")), has_end(t))

    def r_provide_item(self, t):
        return node("pitem", t.children[0].value, self.go(t.children[2]))

    def r_target_expr(self, t):
        return node("target", *self.many(t), has_end(t))

    def r_target_item(self, t):
        c = t.children
        if is_sym(c[0], "else"):
            return node("case-else", self.go(c[1]), has_end(t))
        return node("case", node("req", *self.names(c[1])), self.go(c[2]), has_end(t))

    def r_level_block(self, t):
        c = t.children                       # пометка блок [end пометка]
        end = node("end", c[3].children[0].value) if len(c) > 2 else None
        return node("level-block", c[0].children[0].value, self.go(c[1]), end)

    # ───────── образцы ─────────
    def r_pattern(self, t):
        c = t.children
        if is_sym(c[0], "_"):
            return node("p-wild")
        if c[0].data != "pat_path":
            return self.go(c[0])
        path = node("p-path", *self.names(c[0]))
        if len(c) == 1:
            return path
        a = sub(t, "pattern_args")
        return node("p-ctor", path, *(self.many(a) if a is not None else []))

    def r_pat_literal(self, t):
        c = t.children
        if is_sym(c[0], "-"):
            return node("p-neg-int" if named(c[1], "INT") else "p-neg-float", c[1].value)
        x = c[0]
        if named(x, "STRING"):
            return node("p-str", self.string(x))
        for kind, head in (("INT", "p-int"), ("FLOAT", "p-float"), ("CHAR", "p-char")):
            if named(x, kind):
                return node(head, x.value)
        return node("p-" + x.value)          # true или false

    def r_pattern_arg(self, t):
        c = t.children
        if is_sym(c[0], ".."):
            return node("p-rest")
        if len(c) == 1:
            return self.go(c[0])
        return node("p-field", c[0].value, self.go(c[2]))

    def r_tuple_pattern(self, t):
        return node("p-tuple", *self.many(t))


class Reference:
    """Проверочный разборщик: Lark по grammar/nulix.lark с проверочным лексером."""

    def __init__(self, grammar_dir):
        text = open(os.path.join(grammar_dir, "nulix.lark"), encoding="utf-8").read()
        self.parser = Lark(text, parser="lalr", lexer=nxlex.NxLexer, start=nxlex.STARTS, strict=True,
                           keep_all_tokens=True)
        self.canon = Canon(lambda text: self.parse("expr", text))

    def parse(self, start, text):
        nxlex.EMBEDDED.clear()
        nxlex.EOF_TERM = start in ("file", "stmts")
        return self.parser.parse(text, start=start)

    def run(self, start, text):
        """("OK", запись дерева) или ("ERROR", None). Подстановки ${…} во всех строках разбираются тоже."""
        try:
            return "OK", self.canon.go(self.parse(start, text))
        except (LarkError, nxlex.NxLexError):
            return "ERROR", None


def esc(text):
    return text.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t").replace("\r", "\\r")
