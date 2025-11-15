#pragma once
#include <memory>
#include <llvm/IR/LLVMContext.h>
#include <llvm/IR/Module.h>
#include <llvm/IR/IRBuilder.h>

class Codegen {
    std::unique_ptr<llvm::LLVMContext> context;
    std::unique_ptr<llvm::Module> module;
    std::unique_ptr<llvm::IRBuilder<>> builder;

public:
    Codegen() {
        context = std::make_unique<llvm::LLVMContext>();
        module = std::make_unique<llvm::Module>("nulix", *context);
        builder = std::make_unique<llvm::IRBuilder<>>(*context);
    }

    void generate();
    void run_jit();
};