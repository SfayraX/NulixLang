@echo off
echo === NULIX BUILD — РУЧНАЯ LLVM (Windows prebuilt) ===

:: Проверка ninja
ninja --version >nul 2>&1 || (
    echo ERROR: ninja НЕ НАЙДЕН!
    echo Скачай: https://github.com/ninja-build/ninja/releases/download/v1.12.1/ninja-win.zip
    pause
    exit /b 1
)

:: Проверка LLVM
set "LLVM_DIR=C:\Program Files\LLVM\lib\cmake\llvm"
if not exist "%LLVM_DIR%\LLVMConfigExtensions.cmake" (
    echo ERROR: LLVMConfigExtensions.cmake НЕ НАЙДЕН
    pause
    exit /b 1
)
echo LLVM найден: %LLVM_DIR%

:: Очистка
if exist build rmdir /s /q build
mkdir build
cd build

:: CMake + Ninja + Clang
cmake .. -G Ninja ^
  -DCMAKE_CXX_COMPILER=clang++ ^
  -DCMAKE_C_COMPILER=clang

if %errorlevel% neq 0 (
    echo CMake провалился!
    pause
    exit /b 1
)

:: Сборка
cmake --build .

echo.
echo NULIX УСПЕШНО СОБРАН!
echo Запусти: nulix_compiler.exe ..\test.nx
pause