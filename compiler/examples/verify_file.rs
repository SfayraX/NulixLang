//! Читает IR-файл и проверяет его, печатая время: `verify_file ФАЙЛ.ir`.
//!
//! Из комплекта внешней оценки проверщика (20–21 ноября 2026).
//!
//! Чтение идёт в потоке с огромным стеком, а проверка — на собственном потоке проверщика со
//! стеком 64 МБ (`src/stack.rs`), `ulimit -s` на неё не влияет. Так видно пределы самого
//! проверщика, а не читателя.
//! Модуль после проверки не освобождается (`forget`): глубоко вложенное дерево тоже падает
//! при рекурсивном освобождении, и это отдельный вопрос.

use nulixc::ir_text::parse;
use nulixc::verify::verify;
use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("использование: verify_file ФАЙЛ.ir");
    let text = std::fs::read_to_string(&path).expect("файл не читается");
    let t0 = Instant::now();
    let mut parsed = None;
    for stack in [3usize << 30, 1usize << 30, 256usize << 20] {
        let text = text.clone();
        match std::thread::Builder::new()
            .stack_size(stack)
            .spawn(move || parse(&text))
        {
            Ok(h) => {
                parsed = Some(h.join().expect("чтение упало"));
                break;
            }
            Err(_) => continue,
        }
    }
    let m = match parsed.expect("не удалось создать поток для чтения")
    {
        Ok(m) => m,
        Err(e) => {
            println!("parse: ОШИБКА {:?} за {:?}", e, t0.elapsed());
            return;
        }
    };
    println!("parse: ok за {:?}", t0.elapsed());
    let t1 = Instant::now();
    match verify(&m) {
        Ok(()) => println!("verify: принято за {:?}", t1.elapsed()),
        Err(e) => println!("verify: отвергнуто «{}» за {:?}", e.message, t1.elapsed()),
    }
    std::mem::forget(m);
}
