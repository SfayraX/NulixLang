%{
#include "parser.tab.hpp"
#include <cstdlib>
#include <cstring>
#include <iostream>

#define YY_NO_UNISTD_H
#ifdef _WIN32
extern "C" int isatty(int) { return 0; }
#endif
%}

%option noyywrap
%option nounistd
%option never-interactive

%%

\xEF\xBB\xBF    /* игнорим UTF-8 BOM (VS Code добавляет) */
[\x00-\x1F\x7F] /* игнорим все контрольные символы */
"proc"              { yylval.str = strdup(yytext); return T_PROC; }
"actor"             { yylval.str = strdup(yytext); return T_ACTOR; }
"let"               { yylval.str = strdup(yytext); return T_LET; }
[a-zA-Z_][a-zA-Z0-9_]* { yylval.str = strdup(yytext); return T_ID; }
[0-9]+(\.[0-9]+)?   { yylval.str = strdup(yytext); return T_NUMBER; }
"+"                 { return T_PLUS; }
"-"                 { return T_MINUS; }
"="                 { return T_ASSIGN; }
";"                 { return T_SEMICOLON; }
":"                 { return T_COLON; }
[ \t\n\r]+          /* игнорим пробелы, табы, переносы (добавил \r) */
.                   { std::cerr << "Unknown char: " << yytext << std::endl; }

%%