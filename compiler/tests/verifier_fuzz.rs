//! Дифференциальный фаззер проверщика IR. Из комплекта внешней оценки проверщика (20–21 ноября 2026).
//!
//! Идея: проверщик не вправе принять то, что интерпретатор находит неверным. Фаззер берёт
//! правильный IR из `tests/ir/{data,flow,order,runner}.ir`, портит его случайно по строкам
//! (удалить, продублировать, поменять местами, заменить move/copy/borrow, подменить имя,
//! поправить список `[held: …]`, вставить строку из другого места, сменить константу, вставить
//! или убрать `drop`). Если порченый модуль читается и проверщик его ПРИНЯЛ, он исполняется в
//! интерпретаторе: все тесты модуля и каждая функция без параметров `mut` на трёх наборах
//! значений аргументов. Любая остановка «неверный IR» у принятого модуля — дыра в проверщике.
//! Паника проверщика на разобранном модуле тоже считается ошибкой.
//!
//! Запуск (из каталога `compiler/`):
//!     cargo test --release --test verifier_fuzz -- --nocapture
//! Переменные окружения: FUZZ_N — число порч (по умолчанию 3000 — так фаззер идёт
//! в общем `cargo test` и в проверке на GitHub), FUZZ_SEED — зерно (по
//! умолчанию 1). Прогон из оценки: FUZZ_N=150000 FUZZ_SEED=7. Прогон одинаков от запуска к
//! запуску: генератор псевдослучайных чисел свой, зависимостей нет.
//!
//! Предел метода: оракул — интерпретатор, он видит только исполненные пути и написан по тем же
//! правилам, что и проверщик.

use nulixc::interp::{self, Limits, Stop, Value};
use nulixc::ir::*;
use nulixc::ir_text::parse;
use nulixc::verify::verify;
use std::panic::{catch_unwind, AssertUnwindSafe};

const FILES: [(&str, &str); 4] = [
    ("data", include_str!("ir/data.ir")),
    ("flow", include_str!("ir/flow.ir")),
    ("order", include_str!("ir/order.ir")),
    ("runner", include_str!("ir/runner.ir")),
];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % (n as u64)) as usize
        }
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn has_braces(l: &str) -> bool {
    l.contains('{') || l.contains('}')
}

fn indent(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

/// Переменные функции, в которой стоит строка `i`: параметры и объявленные имена.
fn vars_of(lines: &[String], i: usize) -> Vec<String> {
    let mut start = i;
    while start > 0 && !(lines[start].starts_with("func ") || lines[start].starts_with("test ")) {
        start -= 1;
    }
    let mut out: Vec<String> = Vec::new();
    // параметры
    let h = &lines[start];
    if let (Some(a), Some(b)) = (h.find('('), h.rfind(')')) {
        if a < b && h.starts_with("func ") {
            for part in h[a + 1..b].split(',') {
                if let Some(c) = part.find(':') {
                    let n = part[..c].trim();
                    if !n.is_empty() {
                        out.push(n.to_string());
                    }
                }
            }
        }
    }
    let mut j = start + 1;
    while j < lines.len() && !(lines[j].starts_with('}')) {
        let t = lines[j].trim_start();
        let t = t.strip_prefix("local ").unwrap_or(t);
        if let Some(c) = t.find(':') {
            let n = &t[..c];
            if !n.is_empty() && n.chars().all(is_ident_char) {
                out.push(n.to_string());
            }
        }
        j += 1;
    }
    out.sort();
    out.dedup();
    out
}

/// Заменяет k-е вхождение имени-идентификатора из `names` на `to`.
fn rename_one(line: &str, names: &[String], rng: &mut Rng, pool: &[String]) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut spots: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if is_ident_char(chars[i]) {
            let s = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            let w: String = chars[s..i].iter().collect();
            if names.contains(&w) {
                spots.push((s, i));
            }
        } else {
            i += 1;
        }
    }
    if spots.is_empty() || pool.is_empty() {
        return None;
    }
    let (s, e) = spots[rng.below(spots.len())];
    let to = &pool[rng.below(pool.len())];
    let mut out: String = chars[..s].iter().collect();
    out.push_str(to);
    out.extend(chars[e..].iter());
    Some(out)
}

fn mutate(lines: &mut Vec<String>, rng: &mut Rng) -> bool {
    let body: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            indent(&lines[i]) >= 2
                && !lines[i].trim().is_empty()
                && !lines[i].trim_start().starts_with(';')
        })
        .collect();
    if body.is_empty() {
        return false;
    }
    let i = body[rng.below(body.len())];
    let vars = vars_of(lines, i);
    let kind = rng.below(10);
    match kind {
        0 => {
            if has_braces(&lines[i]) {
                return false;
            }
            lines.remove(i);
        }
        1 => {
            if has_braces(&lines[i]) {
                return false;
            }
            let l = lines[i].clone();
            lines.insert(i, l);
        }
        2 => {
            if i + 1 >= lines.len()
                || has_braces(&lines[i])
                || has_braces(&lines[i + 1])
                || indent(&lines[i + 1]) < 2
            {
                return false;
            }
            lines.swap(i, i + 1);
        }
        3 => {
            let l = lines[i].clone();
            let (from, to): (&str, Vec<&str>) = if l.contains("move ") {
                ("move ", vec!["copy ", "borrow ", "borrow mut "])
            } else if l.contains("borrow mut ") {
                ("borrow mut ", vec!["borrow ", "copy ", "move "])
            } else if l.contains("borrow ") {
                ("borrow ", vec!["borrow mut ", "copy ", "move "])
            } else if l.contains("copy ") {
                ("copy ", vec!["move ", "borrow ", "borrow mut "])
            } else {
                return false;
            };
            let t = to[rng.below(to.len())];
            lines[i] = l.replacen(from, t, 1);
        }
        4 => {
            let l = lines[i].clone();
            match rename_one(&l, &vars, rng, &vars) {
                Some(n) => lines[i] = n,
                None => return false,
            }
        }
        5 => {
            // список held
            let l = lines[i].clone();
            if let Some(a) = l.find("[held:") {
                if let Some(b) = l[a..].find(']') {
                    let inner = &l[a + 6..a + b];
                    let mut items: Vec<String> = inner
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    match rng.below(3) {
                        0 => {
                            if !items.is_empty() {
                                let k = rng.below(items.len());
                                items.remove(k);
                            } else {
                                return false;
                            }
                        }
                        1 => {
                            if vars.is_empty() {
                                return false;
                            }
                            items.push(vars[rng.below(vars.len())].clone());
                        }
                        _ => {
                            if items.is_empty() {
                                return false;
                            }
                            let k = rng.below(items.len());
                            let d = items[k].clone();
                            items.push(d);
                        }
                    }
                    let new_inner = if items.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", items.join(", "))
                    };
                    lines[i] = format!("{}[held:{}{}", &l[..a], new_inner, &l[a + b..]);
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }
        6 => {
            // вставка простой строки из того же модуля
            let simple: Vec<usize> = (0..lines.len())
                .filter(|&k| {
                    indent(&lines[k]) >= 2
                        && !has_braces(&lines[k])
                        && !lines[k].trim_start().starts_with(';')
                })
                .collect();
            if simple.is_empty() {
                return false;
            }
            let src = lines[simple[rng.below(simple.len())]].clone();
            let ind = " ".repeat(indent(&lines[i]));
            lines.insert(i, format!("{}{}", ind, src.trim_start()));
        }
        7 => {
            // числовая константа
            let l = lines[i].clone();
            if let Some(a) = l.find("const ") {
                let rest = &l[a + 6..];
                let end = rest
                    .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.'))
                    .unwrap_or(rest.len());
                if end == 0 {
                    return false;
                }
                let pool = ["0", "1", "2", "-1", "100", "255", "256", "0.5"];
                let rep = pool[rng.below(pool.len())];
                lines[i] = format!("{}const {}{}", &l[..a], rep, &rest[end..]);
            } else {
                return false;
            }
        }
        8 => {
            // вставить drop переменной
            if vars.is_empty() {
                return false;
            }
            let ind = " ".repeat(indent(&lines[i]));
            let v = vars[rng.below(vars.len())].clone();
            lines.insert(i, format!("{}drop {}", ind, v));
        }
        _ => {
            // удалить именно drop
            let drops: Vec<usize> = (0..lines.len())
                .filter(|&k| lines[k].trim_start().starts_with("drop "))
                .collect();
            if drops.is_empty() {
                return false;
            }
            let k = drops[rng.below(drops.len())];
            lines.remove(k);
        }
    }
    true
}

// ───────── значения аргументов ─────────

fn gen(m: &Module, t: &Type, rng: &mut Rng, depth: usize) -> Option<Value> {
    if depth > 4 {
        return None;
    }
    match t {
        Type::Unit => Some(Value::Unit),
        Type::Scalar(s) => Some(match s {
            Scalar::F32 => Value::F32([0.0f32, 1.5, -2.0][rng.below(3)]),
            Scalar::F64 => Value::F64([0.0f64, 1.5, -2.0][rng.below(3)]),
            Scalar::Bool => Value::Bool(rng.below(2) == 0),
            Scalar::Char => Value::Char('a'),
            _ => {
                let (lo, hi) = s.int_range()?;
                let pool: [i128; 7] = [0, 1, 2, 3, -1, lo, hi];
                let mut v = pool[rng.below(pool.len())];
                if v < lo || v > hi {
                    v = 1;
                }
                Value::Int(*s, v)
            }
        }),
        Type::Tuple(ts) => {
            let mut out = Vec::new();
            for t in ts {
                out.push(Some(gen(m, t, rng, depth + 1)?));
            }
            Some(Value::Tuple(out))
        }
        Type::Array(e, n) => {
            if *n > 8 {
                return None;
            }
            let mut out = Vec::new();
            for _ in 0..*n {
                out.push(Some(gen(m, e, rng, depth + 1)?));
            }
            Some(Value::Array(out))
        }
        Type::Named(name) => {
            for d in &m.decls {
                match d {
                    Decl::Struct(s) if &s.name == name => {
                        let mut out = Vec::new();
                        for f in &s.fields {
                            out.push(Some(gen(m, &f.ty, rng, depth + 1)?));
                        }
                        return Some(Value::Struct(name.clone(), out));
                    }
                    Decl::Enum(e) if &e.name == name => {
                        if e.variants.is_empty() {
                            return None;
                        }
                        let vi = rng.below(e.variants.len());
                        let tys: Vec<&Type> = match &e.variants[vi].payload {
                            Payload::None => vec![],
                            Payload::One(t) => vec![t],
                            Payload::Fields(fs) => fs.iter().map(|f| &f.ty).collect(),
                        };
                        let mut out = Vec::new();
                        for t in tys {
                            out.push(Some(gen(m, t, rng, depth + 1)?));
                        }
                        return Some(Value::Enum(name.clone(), vi, out));
                    }
                    _ => {}
                }
            }
            None
        }
    }
}

#[derive(Default)]
struct Stats {
    total: usize,
    mutated: usize,
    parsed: usize,
    verified: usize,
    verify_panics: usize,
    parse_panics: usize,
    run_ok: usize,
    run_crash: usize,
    run_steps: usize,
    invalid_hits: usize,
}

#[test]
fn verifier_accepts_nothing_the_interpreter_finds_invalid() {
    let n: usize = std::env::var("FUZZ_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);
    let seed: u64 = std::env::var("FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let lim = Limits {
        steps: 20_000,
        depth: 60,
    };
    let mut st = Stats::default();
    let _ = std::fs::create_dir_all("/tmp/fuzz_hits");
    let mut seen_msgs: Vec<String> = Vec::new();

    for it in 0..n {
        st.total += 1;
        let (name, text) = FILES[rng.below(FILES.len())];
        let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        let k = 1 + rng.below(3);
        let mut did = 0;
        for _ in 0..k {
            if mutate(&mut lines, &mut rng) {
                did += 1;
            }
        }
        if did == 0 {
            continue;
        }
        st.mutated += 1;
        let mut mtext = lines.join("\n");
        mtext.push('\n');
        let parsed = match catch_unwind(AssertUnwindSafe(|| parse(&mtext))) {
            Ok(p) => p,
            Err(_) => {
                st.parse_panics += 1;
                continue;
            }
        };
        let m = match parsed {
            Ok(m) => m,
            Err(_) => continue,
        };
        st.parsed += 1;
        let v = catch_unwind(AssertUnwindSafe(|| verify(&m)));
        match v {
            Err(_) => {
                st.verify_panics += 1;
                let _ = std::fs::write(format!("/tmp/fuzz_hits/panic_{}_{}.ir", it, name), &mtext);
                continue;
            }
            Ok(Err(_)) => continue,
            Ok(Ok(())) => {}
        }
        st.verified += 1;
        // исполнение
        let mut report = |what: String, msg: String| {
            st.invalid_hits += 1;
            let key = msg.clone();
            if !seen_msgs.contains(&key) {
                seen_msgs.push(key);
                let path = format!("/tmp/fuzz_hits/invalid_{}_{}.ir", it, name);
                let _ = std::fs::write(&path, &mtext);
                println!("ДЫРА: {} -> {} (сохранено {})", what, msg, path);
            }
        };
        match interp::run_tests(&m, lim) {
            Ok(rs) => {
                for r in rs {
                    match r.stop {
                        Some(Stop::InvalidIr(msg)) => report(format!("тест «{}»", r.name), msg),
                        Some(Stop::Crash(_)) => st.run_crash += 1,
                        Some(Stop::OutOfSteps) => st.run_steps += 1,
                        None => st.run_ok += 1,
                    }
                }
            }
            Err(Stop::InvalidIr(msg)) => report("модуль".to_string(), msg),
            Err(_) => {}
        }
        for d in &m.decls {
            if let Decl::Func(f) = d {
                if f.params.iter().any(|p| p.mode == Mode::Mut) {
                    continue;
                }
                for _ in 0..3 {
                    let mut args = Vec::new();
                    let mut okargs = true;
                    for p in &f.params {
                        match gen(&m, &p.ty, &mut rng, 0) {
                            Some(v) => args.push(v),
                            None => {
                                okargs = false;
                                break;
                            }
                        }
                    }
                    if !okargs {
                        break;
                    }
                    match interp::call(&m, &f.name, args, lim) {
                        Err(Stop::InvalidIr(msg)) => report(format!("функция {}", f.name), msg),
                        Err(Stop::Crash(_)) => st.run_crash += 1,
                        Err(Stop::OutOfSteps) => st.run_steps += 1,
                        Ok(_) => st.run_ok += 1,
                    }
                }
            }
        }
    }
    println!(
        "ИТОГ: всего {} | испорчено {} | разобрано {} | принято проверщиком {} | паник проверщика {} | паник чтения {} | исполнено ok {} crash {} шаги {} | ДЫР {}",
        st.total, st.mutated, st.parsed, st.verified, st.verify_panics, st.parse_panics, st.run_ok, st.run_crash, st.run_steps, st.invalid_hits
    );
    assert_eq!(
        st.invalid_hits, 0,
        "проверщик принял IR, который интерпретатор находит неверным"
    );
    assert_eq!(st.verify_panics, 0, "проверщик упал на разобранном модуле");
}
