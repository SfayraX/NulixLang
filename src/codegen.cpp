// src/codegen.cpp — 57 ЖИВ. NULIX ПОБЕДИЛ ВСЁ.
#include "codegen.h"
#include <llvm/Support/TargetSelect.h>
#include <llvm/Support/raw_ostream.h>
#include <llvm/ExecutionEngine/Orc/LLJIT.h>
#include <llvm/ExecutionEngine/Orc/ThreadSafeModule.h>
#include <iostream>

using namespace llvm;
using namespace llvm::orc;

void Codegen::generate() {
    auto* func_type = FunctionType::get(Type::getInt32Ty(*context), false);
    auto* main_func = Function::Create(func_type, Function::ExternalLinkage, "main", module.get());

    auto* entry = BasicBlock::Create(*context, "entry", main_func);
    builder->SetInsertPoint(entry);

    auto* sum = builder->CreateAdd(
        ConstantInt::get(*context, APInt(32, 42)),
        ConstantInt::get(*context, APInt(32, 15))
    );

    FunctionCallee printf_func = module->getOrInsertFunction(
        "printf",
        FunctionType::get(builder->getInt32Ty(), builder->getPtrTy(), true)
    );

    auto* format = builder->CreateGlobalStringPtr("%d\n");
    builder->CreateCall(printf_func, { format, sum });

    builder->CreateRet(ConstantInt::get(*context, APInt(32, 0)));
    module->print(errs(), nullptr);
}

void Codegen::run_jit() {
    InitializeNativeTarget();
    InitializeNativeTargetAsmPrinter();
    InitializeNativeTargetAsmParser();

    auto jit = cantFail(LLJITBuilder().create());

    auto gen = cantFail(
        DynamicLibrarySearchGenerator::GetForCurrentProcess(
            jit->getDataLayout().getGlobalPrefix()
        )
    );

    jit->getMainJITDylib().addGenerator(std::move(gen));  // ← ВОТ ЭТО РАБОТАЕТ

    ThreadSafeModule tsm(std::move(module), ThreadSafeContext(std::move(context)));
    cantFail(jit->addIRModule(std::move(tsm)));

    auto main_sym = cantFail(jit->lookup("main"));
    auto* main_func = main_sym.toPtr<int(*)()>();

    std::cout << "NULIX JIT: 42 + 15 = ";
    std::cout.flush();
    main_func();
    std::cout << "\n57 — ВЫВЕДЕН. NULIX — НЕПОБЕДИМ.\n";
}