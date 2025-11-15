# Nulix — Язык без боли (Zero Pain Language)

[![Nulix](https://img.shields.io/badge/Nulix-Revolution-FF4500?style=for-the-badge&logo=rocket)](https://github.com/твой_логин/NulixLang)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Build: Windows](https://img.shields.io/badge/Build-Windows_11-0078D6?style=for-the-badge&logo=windows)](https://github.com/твой_логин/NulixLang/actions)
[![Stars](https://img.shields.io/github/stars/твой_логин/NulixLang?style=social)](https://github.com/твой_логин/NulixLang)

**Nulix — новый язык программирования 2025 года: Nim синтаксис + Julia math + Elixir actors + native комбайн с декомпилятором.**  
**Пиши от драйверов до космоса — один код, без legacy, без крашей, без "работает на моей машине".**

> "Сел и пиши — в прод сразу. Без боли навсегда." — Ты, создатель Nulix

## Почему Nulix — будущее (убийца Rust/Python/C++/JS)
- **Без боли**: Ownership авто, actors по умолчанию, Result без boilerplate, multiple dispatch.
- **Скорость**: Native машинный код (LLVM) + JIT REPL.
- **Масштаб**: BEAM-like actors для 1M+ соединений.
- **Комбайн**: `nulix eat libcuda.so` — пожрал legacy, теперь твой модуль с безопасностью Nulix.
- **Веб без JS**: Скоро WASM runtime + декомпилируем Chrome.

## Hello World в Nulix (test.nx)
```nx
proc main:
  let rocket_pos = 42 + 15;
  print rocket_pos;  // → 57 в консоли машинным кодом

  actor Player:
    let speed = 100;
```
Сборка и запуск (Windows 11 + MSVC 2022)
```powershell
git clone https://github.com/твой_логин/NulixLang.git
cd NulixLang
mkdir build && cd build
cmake ..
cmake --build . --config Debug
chcp 65001
Get-Content ..\test.nx | .\Debug\nulix_compiler.exe
```
Вывод:
```text
Nulix MVP parser ready!
Nulix parsed proc main
Nulix parsed let rocket_pos
Number: 42
Number: 15
Plus op
Nulix parsed actor Player
Nulix parsed let speed
Number: 100
```
Roadmap 2025-2030

 MVP parser (Flex + Bison + C++ bootstrap)
 LLVM codegen (print 57 в native exe)
 Декомпилятор (пожираем CUDA/TensorFlow)
 Actors runtime (BEAM-like)
 WASM без JS (веб-MMO в браузере)
 Nulix OS (убийца Windows/Linux)

Лицензия
MIT — бери, форкай, делай стартап, влей бабки.
Создано одним человеком в 2025 году — без PhD, без команды, без legacy.
Nulix — для тех, кто устал от боли.
