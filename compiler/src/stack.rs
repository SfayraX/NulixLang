//! Работа на потоке с большим стеком.
//!
//! Разборщик рекурсивный. Его глубина ограничена пределом вложенности, но стека по умолчанию
//! на этот предел может не хватить: в отладочной сборке уровень вложенных блоков стоит
//! десятки килобайт. Поэтому компилятор работает на потоке со стеком `STACK_BYTES`.

/// Размер стека рабочего потока: с запасом в несколько раз на предел вложенности.
pub const STACK_BYTES: usize = 64 * 1024 * 1024;

/// Выполняет работу на потоке с большим стеком и возвращает её результат.
/// Авария внутри работы продолжается в вызывающем потоке.
pub fn run<T, F>(work: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let handle = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(work)
        .expect("не удалось создать рабочий поток");
    match handle.join() {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// То же для работы, которая берёт данные вызывающего по ссылке: поток живёт, пока идёт
/// работа, поэтому данные не копируются. Копия глубоко вложенного значения сама шла бы
/// рекурсивно на стеке вызывающего.
pub fn run_scoped<T, F>(work: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, work)
            .expect("не удалось создать рабочий поток");
        match handle.join() {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })
}
