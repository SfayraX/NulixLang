%{
#include <iostream>
#include <cstring>
#include <cstdio>

extern int yylex();
void yyerror(const char* s) { fprintf(stderr, "Nulix error: %s\n", s); }
%}

%union {
    char* str;
}

%token <str> T_NUMBER T_ID
%token T_PROC T_ACTOR T_LET T_PLUS T_MINUS T_ASSIGN T_SEMICOLON T_COLON

%expect 6

%%

program: stmt_list

stmt_list: /* empty */
         | stmt_list stmt

stmt: proc_def
    | actor_def
    | let_stmt

proc_def: T_PROC T_ID T_COLON stmt_list 
    { printf("Nulix parsed proc %s\n", $2); free($2); }

actor_def: T_ACTOR T_ID T_COLON stmt_list 
    { printf("Nulix parsed actor %s\n", $2); free($2); }

let_stmt: T_LET T_ID T_ASSIGN expr T_SEMICOLON 
    { printf("Nulix parsed let %s\n", $2); free($2); }

expr: term
    | expr T_PLUS term { printf("Plus op\n"); }
    | expr T_MINUS term { printf("Minus op\n"); }

term: T_NUMBER { printf("Number: %s\n", $1); free($1); }
    | T_ID { printf("Var: %s\n", $1); free($1); }

%%