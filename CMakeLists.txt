cmake_minimum_required(VERSION 3.20)
project(nulix_compiler LANGUAGES CXX)

# =============================================================================
# === РУЧНАЯ ЗАГРУЗКА LLVM (Windows prebuilt) ===
# =============================================================================
set(LLVM_INSTALL_PREFIX "C:/Program Files/LLVM")
set(LLVM_CMAKE_DIR "${LLVM_INSTALL_PREFIX}/lib/cmake/llvm")

# Проверяем наличие файла
if(NOT EXISTS "${LLVM_CMAKE_DIR}/LLVMConfigExtensions.cmake")
    message(FATAL_ERROR "LLVMConfigExtensions.cmake НЕ НАЙДЕН в ${LLVM_CMAKE_DIR}")
endif()

# Загружаем вручную
include("${LLVM_CMAKE_DIR}/LLVMConfigExtensions.cmake")

# РУЧНАЯ НАСТРОЙКА ПЕРЕМЕННЫХ (из LLVMConfigExtensions.cmake)
set(LLVM_FOUND TRUE)
set(LLVM_PACKAGE_VERSION "21.1.0")
set(LLVM_INCLUDE_DIRS "${LLVM_INSTALL_PREFIX}/include")
set(LLVM_LIBRARY_DIRS "${LLVM_INSTALL_PREFIX}/lib")
set(LLVM_DEFINITIONS "")

# Ручной список библиотек (для твоего случая)
set(LLVM_AVAILABLE_LIBS 
    LLVMCore
    LLVMOrcJIT
    LLVMX86CodeGen
    LLVMX86AsmParser
    LLVMX86Desc
    LLVMX86Info
    LLVMSupport
    LLVMExecutionEngine
    LLVMRuntimeDyld
    LLVMObject
    LLVMBitReader
    LLVMBitWriter
    LLVMMC
    LLVMMCParser
    LLVMTarget
    LLVMAsmPrinter
    LLVMSelectionDAG
    LLVMCodeGen
    LLVMInstCombine
    LLVMScalarOpts
    LLVMTransformUtils
    LLVMAnalysis
    LLVMProfileData
)

# Формируем пути к .lib
set(llvm_libs)
foreach(lib ${LLVM_AVAILABLE_LIBS})
    set(lib_path "${LLVM_LIBRARY_DIRS}/${lib}.lib")
    if(EXISTS "${lib_path}")
        list(APPEND llvm_libs "${lib_path}")
    else()
        message(WARNING "Библиотека ${lib}.lib НЕ НАЙДЕНА")
    endif()
endforeach()

message(STATUS "Found LLVM ${LLVM_PACKAGE_VERSION}")
message(STATUS "LLVM include: ${LLVM_INCLUDE_DIRS}")
message(STATUS "LLVM libs: ${llvm_libs}")

# =============================================================================
# === FLEX + BISON ===
# =============================================================================
find_package(BISON REQUIRED)
find_package(FLEX REQUIRED)

BISON_TARGET(Parser src/parser.y ${CMAKE_CURRENT_BINARY_DIR}/parser.tab.cpp
             DEFINES_FILE ${CMAKE_CURRENT_BINARYDIR}/parser.tab.hpp)
FLEX_TARGET(Lexer src/lexer.l ${CMAKE_CURRENT_BINARY_DIR}/lexer.cpp)
ADD_FLEX_BISON_DEPENDENCY(Lexer Parser)

# =============================================================================
# === ИСПОЛНЯЕМЫЙ ФАЙЛ ===
# =============================================================================
add_executable(nulix_compiler
    src/main.cpp
    src/codegen.cpp
    ${BISON_Parser_OUTPUTS}
    ${FLEX_Lexer_OUTPUTS}
)

# =============================================================================
# === LLVM: ПОДКЛЮЧЕНИЕ ===
# =============================================================================
target_include_directories(nulix_compiler PRIVATE ${LLVM_INCLUDE_DIRS})
target_link_directories(nulix_compiler PRIVATE ${LLVM_LIBRARY_DIRS})
target_compile_definitions(nulix_compiler PRIVATE ${LLVM_DEFINITIONS})
target_link_libraries(nulix_compiler PRIVATE ${llvm_libs})

# =============================================================================
# === C++20 + ОПЦИИ ===
# =============================================================================
target_compile_features(nulix_compiler PRIVATE cxx_std_20)
target_compile_definitions(nulix_compiler PRIVATE _CRT_SECURE_NO_WARNINGS)

# =============================================================================
# === ГОТОВО ===
# =============================================================================
message(STATUS "nulix_compiler будет собран с LLVM ${LLVM_PACKAGE_VERSION}")