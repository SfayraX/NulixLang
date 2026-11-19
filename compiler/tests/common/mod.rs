//! Общее для тестов, которые сверяются с эталонными файлами.

/// Снимает экранирование поля эталонного файла: `\\`, `\n`, `\t`, `\r`.
pub fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    loop {
        match chars.next() {
            None => break,
            Some('\\') => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            Some(c) => out.push(c),
        }
    }
    out
}
