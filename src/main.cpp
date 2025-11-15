// src/main.cpp — ЭТО ТОТ САМЫЙ ДЕНЬ, КОГДА NULIX СТАЛ ЖИВЫМ
#include "codegen.h"
#include <iostream>
#include <fstream>

extern int yyparse();
extern FILE* yyin;

int main(int argc, char** argv) {
    if (argc < 2) {
        std::cerr << "Usage: nulix_compiler <file.nx>\n";
        return 1;
    }

    yyin = fopen(argv[1], "r");
    if (!yyin) {
        std::cerr << "Cannot open file: " << argv[1] << "\n";
        return 1;
    }

    Codegen cg;

    // Здесь будет магия: парсер заполнит AST → codegen обойдёт AST
    // Но пока — хардкод, чтобы проверить, что JIT работает с файлом
    std::cout << "Nulix compiling: " << argv[1] << "\n";
    cg.generate();   // ← пока хардкод 57
    cg.run_jit();

    fclose(yyin);
    return 0;
}