#!/usr/bin/env python3
"""Пробы пределов проверщика IR. Из комплекта внешней оценки проверщика (20–21 ноября 2026).

Генерирует большие и глубокие IR-модули, прогоняет каждый через `examples/verify_file.rs`
(чтение в потоке с огромным стеком, проверка — на собственном потоке проверщика со стеком 64 МБ) и печатает
время или аварийное завершение.

Что проверяется:
  * вложенные `if` и вложенные циклы — глубина рекурсии проверщика;
  * `switch` по перечислению с очень многими вариантами — рост времени;
  * `switch` по i64, прямой код без ветвлений, много функций — контроль: ожидается линейный рост.

Запуск из корня репозитория, нужен bash:
    python3 tools/verifier/probe_limits.py            # собирает пример и гонит все пробы
    python3 tools/verifier/probe_limits.py --no-build # пример уже собран
Время зависит от машины; сравнивайте отношения (рост при увеличении размера), а не секунды.
"""
import argparse
import os
import subprocess
import sys
import tempfile


def gen_nested_if(d):
    s = ["ir 1", "", "func f(t: bool) {"]
    s += ["if t {"] * d
    s += ["} else { } [held:]"] * d
    s.append("}")
    return "\n".join(s) + "\n"


def gen_nested_loop(d):
    s = ["ir 1", "", "func f(t: bool) {"]
    for i in range(d):
        s.append(f"loop L{i} [held:] {{")
    s.append("break L0")
    for i in reversed(range(d)):
        s.append("} [held:]" if i == 0 else "}")
    s.append("}")
    return "\n".join(s) + "\n"


def gen_enum_switch(k):
    s = ["ir 1", "", "enum E { " + ", ".join(f"V{i}" for i in range(k)) + " }", "", "func f(e: E) {", "switch e {"]
    for i in range(k):
        s.append(f"case V{i} {{ }}")
    s.append("} [held:]")
    s.append("}")
    return "\n".join(s) + "\n"


def gen_int_switch(k):
    s = ["ir 1", "", "func f(x: i64) {", "switch x {"]
    for i in range(k):
        s.append(f"case {i} {{ }}")
    s.append("else { }")
    s.append("} [held:]")
    s.append("}")
    return "\n".join(s) + "\n"


def gen_straight(k):
    s = ["ir 1", "", "func f() {", "x0: i64 = const 0"]
    for i in range(1, k):
        s.append(f"x{i}: i64 = add.i64(copy x{i-1}, const 1)")
    s.append("}")
    return "\n".join(s) + "\n"


def gen_many_funcs(k):
    s = ["ir 1", ""]
    for i in range(k):
        s.append(f"func g{i}() {{ }}")
    s.append("func f() {")
    for i in range(k):
        s.append(f"call g{i}()")
    s.append("}")
    return "\n".join(s) + "\n"


PROBES = [
    ("вложенные if", [1000, 2000, 3000, 4000, 6000, 10000, 50000, 200000], gen_nested_if),
    ("вложенные циклы", [500, 2000], gen_nested_loop),
    ("switch по перечислению", [5000, 20000, 60000], gen_enum_switch),
    ("switch по i64", [5000, 20000, 60000], gen_int_switch),
    ("прямой код", [100000, 400000], gen_straight),
    ("много функций", [20000, 60000], gen_many_funcs),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default=".", help="корень репозитория (там лежит compiler/)")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--timeout", type=int, default=120)
    args = ap.parse_args()
    crate = os.path.join(args.repo, "compiler")
    exe = os.path.join(crate, "target", "release", "examples", "verify_file")
    if not args.no_build:
        r = subprocess.run("cargo build --release --offline --example verify_file", shell=True, cwd=crate)
        if r.returncode != 0:
            sys.exit("пример не собрался")
    with tempfile.TemporaryDirectory() as tmp:
        print(f"{'проба':<26}{'размер':>8}  результат")
        for name, sizes, gen in PROBES:
            for n in sizes:
                path = os.path.join(tmp, "probe.ir")
                with open(path, "w", encoding="utf-8") as f:
                    f.write(gen(n))
                cmd = f"ulimit -s 8192; timeout {args.timeout} '{exe}' '{path}' 2>&1"
                r = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True)
                out = r.stdout.strip().replace("\n", " | ")
                if r.returncode != 0:
                    if "stack overflow" in r.stdout + r.stderr:
                        res = "ПЕРЕПОЛНЕНИЕ СТЕКА (процесс прерван)"
                    elif r.returncode == 124:
                        res = f"дольше {args.timeout} с"
                    else:
                        res = f"аварийное завершение, код {r.returncode}: {out[:80]}"
                else:
                    res = out
                print(f"{name:<26}{n:>8}  {res}")


if __name__ == "__main__":
    main()
