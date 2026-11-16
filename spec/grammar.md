# Nulix — формальная грамматика, версия от 16 ноября 2026

Принято архитектором 15 ноября 2026; 16 ноября 2026 исправлены три ошибки, см. раздел 3. Решения записаны в спецификации, раздел «Формальная грамматика»; этот файл — полный текст правил и итоги проверки. Он собирается скриптом `tools/grammar/build_doc.py` из грамматики и шаблона: править нужно их, а не его.

Грамматика собрана из того, что записано в спецификации (ревизия 247). Всё, чего в спецификации нет, помечено **[П]**: до 15 ноября 2026 в тексте спецификации этого не было, добавлено вместе с грамматикой. Одна грамматика действует на всех уровнях, как и требует спецификация; явное ядро — та же грамматика плюс проверка, что типы, режимы, `try`, `await` и требования написаны явно.

## 1. Что проверено

- **Конфликтов нет.** Генератор разборщиков Lark 1.3.1 собрал грамматику как LALR(1) в строгом режиме, где любой конфликт — ошибка: 119 правил, 51 ключевое слово, 51 знак и пометка. Три заведомо неоднозначные правки грамматики тот же режим отверг, то есть проверка работает.
- **Фрагменты спецификации.** В спецификации нет многострочных примеров кода, только фрагменты в тексте. В разделе «Синтаксис» их 260 разных: 84 — одиночные лексемы, остальные учтены поштучно, неучтённых 0. Вместе с фрагментами из других разделов и 46 примерами к принятым решениям разобрано 229 из 229; 9 пропущены с причиной: названия команд, отдельные знаки, `++` и `--`, которых в языке нет, и `asm(…)`, чьи операнды — открытый вопрос. Неполные фрагменты дополнялись до целой конструкции, например `for x in xs` разбирался как `for x in xs { }`.
- **Тестовые программы.** 3 из 3 разобраны целиком. Их составил Claude из конструкций спецификации; это не код прогонов на примерах — тот в спецификацию не входил.
- **Отрицательные примеры.** 126 из 126 отвергнуты, как и должны: `a and b or c`, `a < b < c`, `a << b + c`, `x++`, `007`, `else` с новой строки и другие.
- **Разборщик компилятора.** Рукописный разборщик на Rust сверяется с этой грамматикой тестами `cargo test` в папке `compiler`: на каждом тексте он обязан принять или отвергнуть его так же, как Lark, и построить такое же дерево. Тексты — 358 примеров, перечисленных выше (232 принимаются, 126 отвергаются), и 3000 текстов, выведенных из правил грамматики (1282 и 1718): среди них есть принятый текст на каждое из 595 раскрытий правил, а отвергаемые получены одной порчей — лишней, пропущенной или переставленной лексемой или частью.
- **Источник.** Фрагменты взяты из ревизии 243 спецификации — последней перед этой работой.

Чего проверка не покрывает: нормализацию имён (NFC), правило одной письменности и запрет управляющих символов направления — в проверочном лексере их нет; соответствие отступов вложенности и порог в 20 строк для меток — это не грамматика.

Разбор идёт без таблицы символов: слово `life` лексер отличает по соседним лексемам, а вопрос «имя в образце — переменная или вариант» решается после разбора.

## 2. Что изменилось после черновика

Архитектор принял черновик и поручил выбрать по каждому решению вариант с наибольшей отдачей при наименьшей цене. Относительно черновика изменено пять вещей:

1. **Ключевые слова.** Остаются зарезервированными, но после точки и после `func` любое слово — имя, поэтому методы могут называться `any`, `take`, `match`, `target`. Слова `end`, `test`, `type` и `discard` стали ключевыми только на своём месте, как `life`: имена `end` и `type` слишком ходовые, чтобы их отнимать.
2. **Образцы для именованных полей.** Поля идут в порядке объявления, имя без двоеточия обязано совпадать с именем поля, переименование и вложенный образец пишутся `left: Num(n)`. Так верны обе фразы спецификации сразу, а перепутать `left` и `right` нельзя. Совпадение имён проверяется после разбора.
3. **Режимы в типе функции.** Параметр в типе функции может иметь имя, и режим пишется перед именем, как в объявлении: `func(mut model: Model, msg: Msg)`. У параметра без имени режима нет, поэтому `func(mut ref T)` читается однозначно.
4. **Метки конца блока.** Добавлены `end case`, `end else`, `end catch`, `end func` и `end @low`: теперь метка есть у каждого вида блока.
5. **Перенос строки.** Строка, которая начинается с точки, продолжает предыдущую, так что цепочку вызовов можно писать по одному вызову на строку.

Остальное принято как в черновике. Не вошло в грамматику и записано в «Открытые вопросы» спецификации: пометка хвостового вызова; объявление параметра-значения, например `N` в `Array[T, N]`; связанные типы интерфейса; `let` без начального значения; объявления функций и типов внутри функций; значение у `break`; доступ к элементу кортежа по номеру; операнды `asm` и тело `@naked`-функции.

## 3. Изменения 16 ноября 2026

Сверка разборщика компилятора с этой грамматикой на текстах, выведенных из её правил, нашла три места, где лексер ломал текст, который грамматика разбирает. Все три исправлены в обоих лексерах — проверочном и компилятора.

1. **Метка `end test`.** Грамматика разрешала её с самого начала, но лексер считал слово `test` ключевым только перед строкой, и метка не разбиралась. Теперь слово `test` ключевое и сразу после `end`. Чтобы конец функции или цикла с именем `test` по-прежнему можно было пометить, правила `end_name` и `end_loop` принимают его и как имя в метке. Это единственное изменение в самих правилах.
2. **Слово после `end func`.** Правило «после `func` любое слово — имя» срабатывало и после метки `end func`, поэтому не разбиралось `func() { … } end func catch { … }`. Теперь метка не в счёт.
3. **Слово `life`.** Лексер считал его ключевым после любой запятой и перед любым словом, поэтому не разбиралось `import m.{a, life as b}`. Теперь оно ключевое только в квадратных скобках, как и записано в решении.

Осталось как есть: функцию, названную ключевым словом, например `func match()`, нельзя пометить `end match` — слово после `end` читается как ключевое.

## 4. Лексика

- **Комментарии.** `//` до конца строки, `///` — документация, `/* … */` вкладываются.
- **Конец инструкции (TERM).** Знак `;` или конец строки, если строка кончается именем, литералом, `)`, `]`, `}`, `_`, одним из слов `return`, `break`, `continue`, `self`, `true`, `false`, `any`, `trust` или меткой после `end`. Внутри `(` и `[` перевод строки ничего не завершает; строка, которая начинается с точки, продолжает предыдущую. В конце файла действует то же правило.
- **Имена (NAME).** По Unicode UAX #31, сравниваются после NFC; `_` допустим в начале имени **[П]**; одиночный `_` — отдельная лексема. Имя `Self` зарезервировано как имя типа, слово `asm` — под ассемблерные вставки **[П]**.
- **Ключевые слова.** `and` `any` `as` `assert` `await` `break` `case` `catch` `const` `continue` `discard` `else` `end` `enum` `extern` `false` `for` `func` `if` `impl` `import` `in` `indirect` `interface` `is` `let` `life` `limit` `loop` `match` `mut` `not` `or` `provide` `ptr` `pub` `ref` `return` `self` `struct` `take` `target` `test` `true` `trust` `try` `type` `uses` `var` `while` `without`. После точки и после `func` любое слово — имя; метка `end func` не в счёт: слово после неё читается как обычно. Слова `end`, `test`, `type`, `discard` и `life` — ключевые только на своём месте: `end` — сразу после `}`, `test` — перед строкой и сразу после `end`, где оно годится и как метка теста, и как имя в метке, `type` — перед именем, `discard` — перед `self`, `life` — в квадратных скобках после `[` или запятой и перед именем, `self` или `(`.
- **Пометки.** `@high @system @low @naked`, `@prototype @develop @production @strict`, `@file` и ATTR — любое другое `@имя`.
- **Целые (INT).** Десятичные с разделителем `_`, `0x…`, `0b…`, `0o…`; ведущий ноль — ошибка; буква сразу после числа — ошибка **[П]**.
- **Вещественные (FLOAT).** `2.5`, `1e9`, `2.5e-3`; цифры обязательны с обеих сторон точки.
- **Строки (STRING).** `"…"` в одну строку, `"""…"""` в несколько; подстановки `$имя` и `${выражение}`; выражение внутри `${…}` разбирается правилом `expr`. Экранирование — `\n \t \r \0 \\ \" \' \$ \u{…}`; `$имя` подставляет только простое имя.
- **Символ (CHAR).** `'a'` — один символ или одно экранирование.

## 5. Приоритеты

От сильного к слабому. Семейства 4а–4д между собой без скобок не смешиваются.

| Уровень | Операторы | Сочетание |
| --- | --- | --- |
| 1 | вызов `f(x)`, индекс `a[i]`, поле `a.b` | слева направо |
| 2 | унарные `-`, `~`, `try`, `await` | справа налево |
| 3 | `catch { … }`, `catch err { … }` | один раз |
| 4а | `*` `/` `%`, затем `+` `-` | слева направо |
| 4б | `<<` `>>` | один раз |
| 4в | `&` | цепочка только из `&` |
| 4г | `\|` | цепочка только из `\|` |
| 4д | `^` | цепочка только из `^` |
| 5 | `..<` `..=` над арифметикой | один раз |
| 6 | `==` `!=` `<` `<=` `>` `>=`, `is` | один раз |
| 7 | `not` | справа налево |
| 8 | `and`; `or` | цепочка из одного слова |

## 6. Грамматика

Текст ниже — тот самый файл, который прошёл проверку, без перепечатки.

```
// Nulix — формальная грамматика. Принята 15 ноября 2026.
// Изменена 16 ноября 2026: слово test в метке после end (правила end_name и end_loop).
// Запись: a b — подряд; a | b — выбор; a? — необязательно; a* — ноль и больше; a+ — один и больше;
// "слово" — лексема как написана; ЗАГЛАВНЫЕ имена — лексемы из лексического раздела.
// TERM — конец инструкции: его ставит лексер в конце строки или знак ";".
// Пометка [П] — в тексте спецификации до 15 ноября 2026 этого не было; добавлено вместе с грамматикой.

%declare NAME INT FLOAT STRING CHAR TERM ATTR

// ───────── Файл ─────────
file: (file_header TERM)? (import_decl TERM)* (top_item TERM)*
file_header: "@file" "(" header_arg ("," header_arg)* ")"
header_arg: NAME | "target" "(" NAME ")"                                  // [П] точная форма заголовка
import_decl: "import" import_path ("as" NAME)?
           | "import" import_path "." "{" import_name ("," import_name)* ","? "}"
import_path: NAME | import_path "." NAME
import_name: NAME ("as" NAME)?                                            // [П] as внутри скобок
top_item: decl | plain_stmt                 // инструкции — только в главном файле @high-скрипта

// ───────── Объявления ─────────
decl: attrs? "pub"? decl_core                                             // [П] пометки стоят перед pub
attrs: (attr TERM?)+
attr: level | strictness | ATTR ("(" args? ")")?
level: "@high" | "@system" | "@low" | "@naked"
strictness: "@prototype" | "@develop" | "@production" | "@strict"
decl_core: binding | func_decl | extern_func | extern_block | struct_decl | enum_decl
         | interface_decl | impl_decl | type_decl | test_decl

binding: ("let" | "var" | "const") bind_target (":" type)? "=" expr
bind_target: NAME | tuple_pattern                                         // [П] разбор кортежа

func_decl: func_sig block end_name?
extern_func: "extern" STRING func_sig block end_name?
func_sig: "func" NAME generics? "(" params? ")" sig_tail?
sig_tail: reqs | "->" closed_type reqs? | "->" open_type
end_name: "end" (NAME | "test")             // после end слово test — всегда ключевое, поэтому названо отдельно

generics: "[" generic_param ("," generic_param)* ","? "]"
generic_param: NAME (":" bound)? | "uses" NAME ("without" req_set)? | "life" NAME
bound: path_type ("and" path_type)*

params: param ("," param)* ","?
param: mode? NAME (":" type)? | mode? "self"
mode: "mut" | "take"

reqs: uses_clause without_clause? limit_clause? | without_clause limit_clause? | limit_clause   // [П] порядок частей
uses_clause: "uses" req_set
without_clause: "without" req_set
limit_clause: "limit" limit_set
req_set: req_item | "(" (req_item ("," req_item)* ","?)? ")"
req_item: req_name | "trust"
req_name: NAME | req_name "." NAME
limit_set: limit_item | "(" limit_item ("," limit_item)* ","? ")"
limit_item: NAME ("<=" | ">=") arith                                      // [П] знак ">="

struct_decl: "struct" NAME generics? "{" fields? "}" end_name?
fields: field (TERM field)* TERM?
field: attrs? "pub"? NAME ":" type ("=" expr)?                            // [П] pub у поля
enum_decl: "enum" NAME generics? "{" variants? "}" end_name?
variants: variant (TERM variant)* TERM?
variant: "indirect"? NAME variant_fields?                                 // [П] место слова indirect
variant_fields: "(" type ")" | "(" named_field ("," named_field)* ","? ")"
named_field: NAME ":" type
interface_decl: "interface" NAME generics? (":" bound)? "{" members? "}" end_name?
impl_decl: "impl" generics? path_type ("for" path_type)? "{" members? "}" end_impl?
end_impl: "end" "impl" type_name ("for" type_name)?
members: member (TERM member)* TERM?
member: attrs? "pub"? (func_sig | func_decl | extern_func | binding)
type_decl: "type" NAME generics? "=" type
test_decl: "test" STRING block ("end" "test")?                            // [П] метка end test
extern_block: "extern" STRING "trust" STRING "{" members? "}" ("end" "extern")?   // [П] метка end extern

// ───────── Типы ─────────
// closed_type и open_type различаются только тем, чем тип кончается: open_type кончается типом функции,
// и идущие следом uses, without и limit относятся к этой функции, а не к внешней сигнатуре.
type: closed_type | open_type
closed_type: simple_type | "(" ")" | "(" type ")" | "(" type ("," type)+ ","? ")"    // [П] скобки и тип-кортеж
simple_type: path_type | "any" path_type | "any"
           | "ref" origin? simple_type | "mut" "ref" origin? simple_type
           | "ptr" simple_type | "mut" "ptr" simple_type
open_type: fn_type
         | "ref" origin? open_type | "mut" "ref" origin? open_type
         | "ptr" open_type | "mut" "ptr" open_type
origin: "(" (NAME | "self") ")"                                      // [П] self как источник
path_type: type_name generic_args?
type_name: NAME | type_name "." NAME
generic_args: "[" generic_arg ("," generic_arg)* ","? "]"
generic_arg: type | INT | "uses" req_set | "life" life_src
life_src: NAME | "self" | "(" NAME ("," NAME)* ")"
fn_type: fn_kind? "func" "(" fn_params? ")" fn_tail?
fn_params: fn_param ("," fn_param)* ","?
fn_param: type | mode? (NAME | "_") ":" type                              // [П] имя и режим параметра
fn_kind: "mut" | "take" | "any" | "extern" STRING
fn_tail: reqs | "->" closed_type reqs? | "->" open_type

// ───────── Инструкции ─────────
block: "{" stmts? "}"
stmts: stmt (TERM stmt)* TERM?
stmt: binding | plain_stmt
plain_stmt: expr | assign | for_stmt | while_stmt | loop_stmt
          | "return" expr? | "break" NAME? | "continue" NAME?
          | "assert" expr ("," expr)? | "discard" "self"
assign: postfix assign_op expr
assign_op: "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>="   // [П] список составных
for_stmt: label? "for" for_target "in" mode? expr block end_loop?
while_stmt: label? "while" expr block end_loop?
loop_stmt: label? "loop" block end_loop?
label: NAME ":"
for_target: NAME | "_" | tuple_pattern                                    // [П] _ и кортеж
end_loop: "end" ("for" | "while" | "loop" | NAME | "test")                // [П] end while, end loop

// ───────── Выражения ─────────
// Приоритеты — частичный порядок [П]: and и or не смешиваются; сравнения не выстраиваются в цепочку;
// арифметика, сдвиги, &, | и ^ — отдельные семейства, между собой только через скобки.
expr: or_expr | and_expr | not_expr
or_expr: not_expr ("or" not_expr)+
and_expr: not_expr ("and" not_expr)+
not_expr: "not" not_expr | comparison
comparison: operand | operand cmp_op operand | operand "is" pattern
cmp_op: "==" | "!=" | "<" | "<=" | ">" | ">="                             // [П] != > >=
operand: arith | range | bit_and | bit_or | bit_xor | shift
range: arith "..<" arith | arith "..=" arith
arith: term | arith "+" term | arith "-" term
term: unary | term "*" unary | term "/" unary | term "%" unary            // [П] + - * / % не названы
bit_and: unary "&" unary | bit_and "&" unary
bit_or: unary "|" unary | bit_or "|" unary
bit_xor: unary "^" unary | bit_xor "^" unary
shift: unary "<<" unary | unary ">>" unary
unary: prefix | prefix "catch" NAME? block ("end" "catch")?                // [П] метка end catch
prefix: postfix | "-" prefix | "~" prefix | "try" prefix | "await" prefix
postfix: primary | postfix "." NAME | postfix "(" args? ")" | postfix "[" index_args "]"
primary: NAME | "self" | literal
       | "(" expr ")" | "(" ")" | "(" expr ("," expr)+ ","? ")"            // [П] значение-кортеж
       | "[" (expr ("," expr)* ","?)? "]"
       | if_expr | match_expr | closure | trust_expr | provide_expr | target_expr | level_block
literal: INT | FLOAT | STRING | CHAR | "true" | "false"                    // [П] true и false
args: arg ("," arg)* ","?
arg: expr | "mut" expr | NAME ":" expr
index_args: index_arg ("," index_arg)* ","?
index_arg: expr | "any" path_type | "any" | "uses" req_set                // [П] типы в индексных скобках

if_expr: if_chain ("end" "if")?
if_chain: "if" expr block | "if" expr block "else" block | "if" expr block "else" if_chain
match_expr: "match" mode? expr "{" (match_item TERM?)* "}" ("end" "match")?
match_item: "case" pattern ("," pattern)* ("if" expr)? block ("end" "case")?   // [П] метки end case, end else
          | "else" block ("end" "else")?
closure: "func" "(" params? ")" sig_tail? block ("end" "func")?           // [П] результат, требования, метка
trust_expr: "trust" STRING block ("end" "trust")?                         // [П] метка end trust
provide_expr: "provide" provide_set block ("end" "provide")?              // [П] метка end provide
provide_set: provide_item | "(" provide_item ("," provide_item)* ","? ")"
provide_item: NAME "=" expr
target_expr: "target" "{" (target_item TERM?)* "}" ("end" "target")?      // [П] метка end target
target_item: "case" req_name block ("end" "case")? | "else" block ("end" "else")?   // [П] условие — одно имя
level_block: (level | strictness) block ("end" (level | strictness))?    // [П] метка end @low

// ───────── Образцы ─────────
pattern: "_" | pat_literal | pat_path | pat_path "(" pattern_args? ")" | tuple_pattern
pat_literal: INT | FLOAT | STRING | CHAR | "true" | "false" | "-" INT | "-" FLOAT   // [П] отрицательные числа
pat_path: NAME | pat_path "." NAME
pattern_args: pattern_arg ("," pattern_arg)* ","?
pattern_arg: pattern | NAME ":" pattern | ".."                            // [П] форма «имя: образец»
tuple_pattern: "(" pattern ("," pattern)+ ","? ")"                        // [П] образец-кортеж
```

Ограничения, которые грамматика не выражает и которые проверяются после разбора: тело функции обязательно везде, кроме интерфейса и блока `extern`; в блоке `extern` тел нет; `else` в `match` и `target` стоит последним; `..` в образце стоит последним; импорты идут только в начале файла (это грамматика выражает), инструкции верхнего уровня допустимы только в главном файле @high-скрипта; метка после `end` совпадает с началом блока.
