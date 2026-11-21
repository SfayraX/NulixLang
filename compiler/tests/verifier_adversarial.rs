//! Точечные программы против проверщика IR. Из комплекта внешней оценки проверщика (20–21 ноября 2026).
//!
//! Восемнадцать программ из оценки (c1–c12, w1–w6) и две добавленные при подготовке комплекта
//! (c8b, c8c): программа c8 из оценки оказалась составлена неверно — первый `move e.A.0` стоит
//! до `switch`, поэтому проверщик отвергает её по постороннему поводу; c8b и c8c проверяют то,
//! что c8 должна была проверять (второе перемещение уже перемещённого содержимого варианта).
//!
//! Для каждой программы записано ожидание: принимает ли проверщик, находит ли интерпретатор
//! неверный IR при исполнении её теста и — для отвергнутых — какая причина названа в сообщении
//! проверщика (чтобы отказ не получился по случайному поводу).
//!
//! Главное свойство — безопасность: принятое проверщиком интерпретатор не вправе считать неверным.
//!
//! Запуск (из каталога `compiler/`):
//!     cargo test --test verifier_adversarial -- --nocapture

use nulixc::interp::{self, Limits, Stop};
use nulixc::ir_text::parse;
use nulixc::verify::verify;

/// Общая шапка программ c1–c12.
const PRE_C: &str = r##"ir 1

struct Token { id: i64 }
enum E2 { A(Token), B(Token), C }
copy enum D { L(i64), R(i64) }
copy struct Q { d: D, n: i64 }

func flip(q: Q mut) { q.d = make D.R(const 5) }
func ident(t: Token take) -> Token { return move t }
func bump(i: i64 mut) { i = add.i64(copy i, const 1) }
"##;

/// Общая шапка программ w1–w6.
const PRE_W: &str = r##"ir 1

struct Token { id: i64 }
enum E2 { A(Token), B(Token), C }
copy enum D { L(i64), R(i64) }
"##;

// ───────── c1–c6: правило «известного варианта» ─────────

/// Вызов с `borrow mut` над объемлющим местом может поменять вариант: факт должен погаснуть.
const C1: &str = r##"
func c1() {
  d0: D = make D.L(const 1)
  q: Q = make Q(d: copy d0, n: const 0)
  switch q.d {
    case L {
      call flip(borrow mut q)
      x: i64 = copy q.d.L.0
    }
    case R { }
  } [held:]
}
test "c1" { call c1() }
"##;

/// Тело цикла меняет вариант: на втором проходе чтение варианта L уже неверно.
const C2: &str = r##"
func c2() {
  d0: D = make D.L(const 1)
  q: Q = make Q(d: copy d0, n: const 0)
  switch q.d {
    case L {
      i: i64 = const 0
      loop W [held:] {
        x: i64 = copy q.d.L.0
        q.d = make D.R(const 5)
        i = add.i64(copy i, const 1)
        t: bool = ge.i64(copy i, const 2)
        if t { break W } else { } [held:]
      } [held:]
    }
    case R { }
  } [held:]
}
test "c2" { call c2() }
"##;

/// Запись в одной из двух веток `if` делает вариант неизвестным после слияния.
const C3: &str = r##"
func c3(t: bool) {
  d0: D = make D.L(const 1)
  q: Q = make Q(d: copy d0, n: const 0)
  switch q.d {
    case L {
      if t { q.d = make D.R(const 5) } else { } [held:]
      x: i64 = copy q.d.L.0
    }
    case R { }
  } [held:]
}
test "c3" { call c3(const true) }
"##;

/// Переменная, дающая номер элемента, переписана: факт про `arr[i]` теряется.
const C4: &str = r##"
func c4() {
  d0: D = make D.L(const 1)
  d1: D = make D.R(const 2)
  arr: Array[D, 2] = array(copy d0, copy d1)
  i: i64 = const 0
  switch arr[i] {
    case L {
      i = const 1
      x: i64 = copy arr[i].L.0
    }
    case R { }
  } [held:]
}
test "c4" { call c4() }
"##;

/// То же, но номер меняет вызов с `borrow mut i`.
const C5: &str = r##"
func c5() {
  d0: D = make D.L(const 1)
  d1: D = make D.R(const 2)
  arr: Array[D, 2] = array(copy d0, copy d1)
  i: i64 = const 0
  switch arr[i] {
    case L {
      call bump(borrow mut i)
      x: i64 = copy arr[i].L.0
    }
    case R { }
  } [held:]
}
test "c5" { call c5() }
"##;

/// Запись в `arr[j]`, где `j` может совпасть с `i`, меняет тот же элемент.
const C6: &str = r##"
func c6(j: i64) {
  d0: D = make D.L(const 1)
  d1: D = make D.L(const 2)
  arr: Array[D, 2] = array(copy d0, copy d1)
  i: i64 = const 0
  switch arr[i] {
    case L {
      arr[j] = make D.R(const 9)
      x: i64 = copy arr[i].L.0
    }
    case R { }
  } [held:]
}
test "c6" { call c6(const 0) }
"##;

// ───────── c7–c12: частичные перемещения, заимствования ─────────

/// Перезапись перечисления, содержимое которого уже перемещено: принимается (терять нечего).
const C7: &str = r##"
func c7() {
  tk: Token = make Token(id: const 1)
  e: E2 = make E2.A(move tk)
  switch e {
    case A {
      t: Token = move e.A.0
      drop t
      e = make E2.C
    }
    case B {
      drop e
      e = make E2.C
    }
    case C { }
  } [held: e]
  drop e
}
test "c7" { call c7() }
"##;

/// ВНИМАНИЕ: составлена неверно. Первый `move e.A.0` стоит до `switch`, поэтому проверщик
/// отвергает программу из-за него («вариант здесь неизвестен»), а не из-за того, что должна
/// проверять. Оставлена как есть, потому что вошла в оценку; настоящая проверка — c8b и c8c.
const C8: &str = r##"
func c8() {
  tk: Token = make Token(id: const 1)
  e: E2 = make E2.A(move tk)
  t: Token = move e.A.0
  switch e {
    case A {
      u: Token = move e.A.0
      drop u
    }
    case B { drop e }
    case C { }
  } [held:]
  drop t
}
test "c8" { call c8() }
"##;

/// Повторное перемещение содержимого варианта в той же ветке.
const C8B: &str = r##"
func c8b() {
  tk: Token = make Token(id: const 1)
  e: E2 = make E2.A(move tk)
  switch e {
    case A {
      t: Token = move e.A.0
      drop t
      u: Token = move e.A.0
      drop u
    }
    case B { drop e }
    case C { }
  } [held:]
}
test "c8b" { call c8b() }
"##;

/// Второй `switch` по уже частично перемещённому значению и перемещение в его ветке.
const C8C: &str = r##"
func c8c() {
  tk: Token = make Token(id: const 1)
  e: E2 = make E2.A(move tk)
  switch e {
    case A {
      t: Token = move e.A.0
      drop t
      switch e {
        case A {
          u: Token = move e.A.0
          drop u
        }
        case B { drop e }
        case C { }
      } [held:]
    }
    case B { drop e }
    case C { }
  } [held:]
}
test "c8c" { call c8c() }
"##;

/// Объявление переменной заново из вызова, который забрал её же значение: принимается.
const C9: &str = r##"
func c9() {
  tk: Token = make Token(id: const 1)
  tk: Token = call ident(move tk)
  drop tk
}
test "c9" { call c9() }
"##;

/// Запись в часть значения, которое вызов только что забрал целиком.
const C10: &str = r##"
struct Pair { a: Token, b: Token }
func take_pair(p: Pair take) -> Token { a: Token = move p.a
 b: Token = move p.b
 drop b
 drop p
 return move a }
func c10() {
  t1: Token = make Token(id: const 1)
  t2: Token = make Token(id: const 2)
  p: Pair = make Pair(a: move t1, b: move t2)
  p.a = call take_pair(move p)
  drop p
}
test "c10" { call c10() }
"##;

/// Перемещение содержимого варианта в цикле: на втором проходе оно уже пусто.
const C11: &str = r##"
func c11() {
  tk: Token = make Token(id: const 1)
  e: E2 = make E2.A(move tk)
  i: i64 = const 0
  loop W [held: e, i] {
    switch e {
      case A {
        t: Token = move e.A.0
        drop t
      }
      case B { }
      case C { }
    } [held: e, i]
    i = add.i64(copy i, const 1)
    c: bool = ge.i64(copy i, const 2)
    if c { break W } else { } [held: e, i]
  } [held: e, i]
  drop e
}
test "c11" { call c11() }
"##;

/// `borrow mut arr[i]` и `borrow mut i` в одном вызове: принимается, на исполнении всё в порядке.
const C12: &str = r##"
func two(a: i64 mut, b: i64 mut) { b = add.i64(copy b, const 1) }
func c12() {
  arr: Array[i64, 3] = array(const 10, const 20, const 30)
  i: i64 = const 0
  call two(borrow mut arr[i], borrow mut i)
  x: i64 = copy arr[0]
  y: i64 = copy arr[1]
}
test "c12" { call c12() }
"##;

// ───────── w1–w6 ─────────

/// Запись в содержимое другого варианта (простой тип).
const W1: &str = r##"
func w1() {
  d0: D = make D.L(const 1)
  switch d0 {
    case L { d0.R.0 = const 5 }
    case R { }
  } [held:]
}
test "w1" { call w1() }
"##;

/// То же для непростого содержимого.
const W2: &str = r##"
func w2() {
  tk: Token = make Token(id: const 1)
  e: E2 = make E2.A(move tk)
  switch e {
    case A {
      t2: Token = make Token(id: const 2)
      e.B.0 = move t2
    }
    case B { drop e }
    case C { }
  } [held:]
}
test "w2" { call w2() }
"##;

/// Запись в содержимое варианта, когда вариант неизвестен: проверщик строже интерпретатора.
const W3: &str = r##"
func w3(x: D take) {
  x.L.0 = const 5
}
test "w3" { d0: D = make D.L(const 1)
  call w3(move d0) }
"##;

/// Метка `switch` на нижней границе знакового типа: принимается.
const W4: &str = r##"
func w4(x: i8) -> i8 {
  switch x {
    case -128 { return const 1 }
    else { return const 0 }
  }
}
test "w4" { r: i8 = call w4(const -128) }
"##;

/// Метка `switch` на нижней границе беззнакового типа: принимается.
const W5: &str = r##"
func w5(x: u8) -> u8 {
  switch x {
    case 0 { return const 1 }
    else { return const 0 }
  }
}
test "w5" { r: u8 = call w5(const 0) }
"##;

/// Логические и побитовые операции: принимаются.
const W6: &str = r##"
func w6() {
  a: bool = const true
  b: bool = const false
  c: bool = and.bool(copy a, copy b)
  d: bool = not.bool(copy c)
  e: u8 = const 6
  f: u8 = xor.u8(copy e, const 3)
}
test "w6" { call w6() }
"##;

struct Case {
    name: &'static str,
    prelude: &'static str,
    body: &'static str,
    /// принимает ли проверщик
    accepted: bool,
    /// находит ли интерпретатор неверный IR при исполнении теста
    interp_invalid: bool,
    /// подстрока сообщения проверщика (для отвергнутых)
    reason: &'static str,
}

const SAFE_VARIANT: &str = "здесь неизвестен";

fn cases() -> Vec<Case> {
    let c = |name, prelude, body, accepted, interp_invalid, reason| Case {
        name,
        prelude,
        body,
        accepted,
        interp_invalid,
        reason,
    };
    vec![
        c(
            "c1_call_ancestor",
            PRE_C,
            C1,
            false,
            true,
            "вариант в q.d здесь неизвестен",
        ),
        c(
            "c2_loop_flip",
            PRE_C,
            C2,
            false,
            true,
            "вариант в q.d здесь неизвестен",
        ),
        c(
            "c3_branch_flip",
            PRE_C,
            C3,
            false,
            true,
            "вариант в q.d здесь неизвестен",
        ),
        c(
            "c4_index_written",
            PRE_C,
            C4,
            false,
            true,
            "вариант в arr[i] здесь неизвестен",
        ),
        c(
            "c5_index_via_call",
            PRE_C,
            C5,
            false,
            true,
            "вариант в arr[i] здесь неизвестен",
        ),
        c(
            "c6_alias_elem",
            PRE_C,
            C6,
            false,
            true,
            "вариант в arr[i] здесь неизвестен",
        ),
        c("c7_enum_overwrite_after_move", PRE_C, C7, true, false, ""),
        c("c8_composed_wrongly", PRE_C, C8, false, true, SAFE_VARIANT),
        c(
            "c8b_second_move_in_case",
            PRE_C,
            C8B,
            false,
            true,
            "значения нет или часть его перемещена",
        ),
        c(
            "c8c_nested_switch_after_move",
            PRE_C,
            C8C,
            false,
            true,
            "значения нет или часть его перемещена",
        ),
        c("c9_define_over_taken", PRE_C, C9, true, false, ""),
        c(
            "c10_assign_part_after_whole_move",
            PRE_C,
            C10,
            false,
            true,
            "нет значения, частью которого оно было бы",
        ),
        c(
            "c11_loop_partial_move",
            PRE_C,
            C11,
            false,
            true,
            "записанное состояние называет e целиком, а часть значения перемещена",
        ),
        c("c12_lend_elem_and_index", PRE_C, C12, true, false, ""),
        c(
            "w1_write_other_variant_copy",
            PRE_W,
            W1,
            false,
            true,
            "в d0 вариант L",
        ),
        c(
            "w2_write_other_variant_noncopy",
            PRE_W,
            W2,
            false,
            true,
            "в e вариант A",
        ),
        c(
            "w3_write_payload_unknown_variant",
            PRE_W,
            W3,
            false,
            false,
            "вариант в x здесь неизвестен",
        ),
        c("w4_i8_min_label", PRE_W, W4, true, false, ""),
        c("w5_u8_zero_label", PRE_W, W5, true, false, ""),
        c("w6_bool_and_bit_ops", PRE_W, W6, true, false, ""),
    ]
}

#[test]
fn adversarial_programs() {
    let mut problems: Vec<String> = Vec::new();
    println!(
        "{:<36} {:<12} {:<10}",
        "программа", "проверщик", "интерпретатор"
    );
    for c in cases() {
        let text = format!("{}{}", c.prelude, c.body);
        let m = match parse(&text) {
            Ok(m) => m,
            Err(e) => {
                problems.push(format!("{}: программа не читается: {:?}", c.name, e));
                continue;
            }
        };
        let v = verify(&m);
        let invalid = match interp::run_tests(&m, Limits::default()) {
            Ok(rs) => rs
                .iter()
                .any(|r| matches!(r.stop, Some(Stop::InvalidIr(_)))),
            Err(Stop::InvalidIr(_)) => true,
            Err(_) => false,
        };
        println!(
            "{:<36} {:<12} {:<10}",
            c.name,
            if v.is_ok() {
                "принято"
            } else {
                "отвергнуто"
            },
            if invalid {
                "неверный IR"
            } else {
                "исполнено"
            }
        );
        // 1. безопасность
        if v.is_ok() && invalid {
            problems.push(format!(
                "{}: ДЫРА — проверщик принял, а интерпретатор находит неверный IR",
                c.name
            ));
        }
        // 2. ожидания
        if v.is_ok() != c.accepted {
            problems.push(format!(
                "{}: ожидалось «{}», получено «{}»",
                c.name,
                if c.accepted {
                    "принято"
                } else {
                    "отвергнуто"
                },
                if v.is_ok() {
                    "принято"
                } else {
                    "отвергнуто"
                }
            ));
        }
        if invalid != c.interp_invalid {
            problems.push(format!(
                "{}: интерпретатор: ожидалось неверный IR = {}, получено {}",
                c.name, c.interp_invalid, invalid
            ));
        }
        if let Err(e) = &v {
            if !c.reason.is_empty() && !e.message.contains(c.reason) {
                problems.push(format!(
                    "{}: причина отказа другая: «{}» (ожидалось «…{}…»)",
                    c.name, e.message, c.reason
                ));
            }
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
