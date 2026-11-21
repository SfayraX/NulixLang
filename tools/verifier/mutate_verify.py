#!/usr/bin/env python3
"""Мутационная проверка тестов проверщика IR. Из комплекта внешней оценки проверщика (20–21 ноября 2026).

Портит `compiler/src/verify.rs` по одной правке за раз и запускает `cargo test --test verify`.
Мутант «убит», если хотя бы один тест упал; «выжил», если все тесты прошли, — значит, правку
тесты не заметили (пробел покрытия либо эквивалентный мутант: правка не меняет поведения).

Два режима:
  checks     — обезвредить каждую проверку вида `if УСЛОВИЕ {` с `return self.fail(...)`
               (или `decl_err`, `Err`) в первых строках тела: `if false && (УСЛОВИЕ) {`.
               Берутся только однострочные условия; на коммите 2e295eb таких 44.
  operators  — один раз на строку заменить `&&`↔`||`, `==`↔`!=`, `.any(`↔`.all(`,
               `<=`→`<`, `>=`→`>`; строки со строковыми литералами и первые 340 строк
               (объявления) пропускаются; на коммите 2e295eb таких 89.

Запуск из корня репозитория на ЧИСТОЙ копии (скрипт правит verify.rs на месте и возвращает
его; перед правкой делает копию `verify.rs.orig` и при обрыве восстанавливает её при
следующем запуске):
    python3 tools/verifier/mutate_verify.py checks
    python3 tools/verifier/mutate_verify.py operators
Если оболочка ограничивает время, задайте --budget СЕКУНД: скрипт остановится, сохранит
прогресс в файл состояния и продолжит при повторном запуске той же командой.
Прогон детерминирован: перебираются все кандидаты по порядку строк, выборки нет.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time

OPS = [(" && ", " || "), (" || ", " && "), (" == ", " != "), (" != ", " == "),
       (".any(", ".all("), (".all(", ".any("), (" <= ", " < "), (" >= ", " > ")]


def cands_checks(lines):
    out = []
    for i, l in enumerate(lines):
        if re.match(r"^\s*if [^l].*\{\s*$", l) and not re.match(r"^\s*if let", l):
            nxt = " ".join(x.strip() for x in lines[i + 1:i + 4])
            if "return self.fail(" in nxt[:60] or "return decl_err(" in nxt[:60] or "return Err(" in nxt[:40]:
                m = re.match(r"^(\s*)if (.*) \{\s*$", l)
                out.append((f"{i}", i, f"{m.group(1)}if false && ({m.group(2)}) {{"))
    return out


def cands_operators(lines):
    out = []
    for i, l in enumerate(lines):
        if l.strip().startswith("//") or '"' in l or i < 340:
            continue
        for a, b in OPS:
            if a in l:
                out.append((f"{i}:{a}", i, l.replace(a, b, 1)))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["checks", "operators"])
    ap.add_argument("--repo", default=".")
    ap.add_argument("--budget", type=float, default=1e9)
    ap.add_argument("--state", default=None)
    ap.add_argument("--test-cmd", default="cargo test --offline --test verify")
    args = ap.parse_args()
    crate = os.path.join(args.repo, "compiler")
    src = os.path.join(crate, "src", "verify.rs")
    bak = src + ".orig"
    state_path = args.state or os.path.join(args.repo, f".mutate_{args.mode}.json")
    if os.path.exists(bak):  # прошлый запуск оборвали: вернуть исходник
        shutil.copyfile(bak, src)
        os.remove(bak)
    orig = open(src, encoding="utf-8").read()
    lines = orig.split("\n")
    cands = (cands_checks if args.mode == "checks" else cands_operators)(lines)
    state = json.load(open(state_path)) if os.path.exists(state_path) else {}
    # Базовый прогон: на неиспорченном коде тесты обязаны проходить, иначе все мутанты «убиты» впустую.
    r0 = subprocess.run(args.test_cmd, shell=True, cwd=crate, capture_output=True, text=True, timeout=600)
    if r0.returncode != 0:
        tail = "\n".join((r0.stdout + "\n" + r0.stderr).strip().split("\n")[-15:])
        sys.exit("базовый прогон не проходит — мутации бессмысленны. Запускайте из корня полного клона "
                 "репозитория (тестам нужен spec/ir-core.md).\n" + tail)
    shutil.copyfile(src, bak)
    t0 = time.time()
    try:
        for key, i, new_line in cands:
            if key in state:
                continue
            if time.time() - t0 > args.budget:
                break
            mutated = list(lines)
            mutated[i] = new_line
            with open(src, "w", encoding="utf-8") as f:
                f.write("\n".join(mutated))
            try:
                r = subprocess.run(args.test_cmd, shell=True, cwd=crate, capture_output=True, text=True, timeout=300)
                out = r.stdout + "\n" + r.stderr  # вывод целиком: обрезка теряет строку «test result»
                if r.returncode == 0:
                    res = "ВЫЖИЛ"          # cargo test вернул 0: собралось, все тесты прошли
                elif "could not compile" in out or "error[" in out:
                    res = "не собралось"
                else:
                    res = "убит"           # собралось, но тест упал
            except subprocess.TimeoutExpired:
                res = "убит"  # тесты зависли — правка замечена
            state[key] = [res, i + 1, lines[i].strip()]
            json.dump(state, open(state_path, "w"))
    finally:
        shutil.copyfile(bak, src)
        os.remove(bak)
    done = [v for k, v in state.items() if k in {c[0] for c in cands}]
    print(f"режим {args.mode}: кандидатов {len(cands)} | проверено {len(done)} | убито {sum(v[0] == 'убит' for v in done)}"
          f" | ВЫЖИЛО {sum(v[0] == 'ВЫЖИЛ' for v in done)} | не собралось {sum(v[0] == 'не собралось' for v in done)}")
    if len(done) < len(cands):
        print("не закончено: повторите команду, прогресс сохранён в", state_path)
    for v in sorted((v for v in done if v[0] == "ВЫЖИЛ"), key=lambda v: v[1]):
        print(f"  выжил: строка {v[1]}: {v[2]}")


if __name__ == "__main__":
    main()
