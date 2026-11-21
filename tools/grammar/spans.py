"""Выписывает из спецификации все фрагменты в обратных кавычках и место каждого:
номер пункта (12.19), номер раздела для текста вне пунктов (13) или «-» для шапки
до первого раздела. По этому списку check.py проверяет, что учтён каждый фрагмент
спецификации. Список пересобирается после каждой правки спецификации;
проверка на GitHub пересобирает его и сверяет с лежащим в репозитории.
Запуск: python3 tools/grammar/spans.py spec/nulix.md grammar/spans.tsv
"""
import re, sys

# Разметка — как в tools/spec/numbers.py: «## 12. Синтаксис», «- **12.19.** текст»,
# у открытого вопроса — «- [ ] **16.3.** текст».
SECTION = re.compile(r"^## (\d+)\. ")
ITEM = re.compile(r"^- (\[[ x]\] )?\*\*(\d+\.\d+)\.\*\* ")
FRAGMENT = re.compile(r"`([^`]*)`")

src, out = sys.argv[1], sys.argv[2]
rows, section = [], "-"
for n, line in enumerate(open(src, encoding="utf-8").read().split("\n"), 1):
    # Блоков кода, двойных и непарных обратных кавычек в спецификации нет; если появятся,
    # фрагменты здесь выделятся неверно, поэтому список не строится.
    if line.startswith("```") or "``" in line or line.count("`") % 2:
        sys.exit(f"строка {n}: блок кода или непарная обратная кавычка — spans.py их не разбирает")
    m = SECTION.match(line)
    if m:
        section = m.group(1)
    m = ITEM.match(line)
    place = m.group(2) if m else section
    for f in FRAGMENT.finditer(line):
        if "\t" in f.group(1):
            sys.exit(f"строка {n}: табуляция во фрагменте {f.group(1)!r}")
        rows.append((place, f.group(1)))
with open(out, "w", encoding="utf-8") as f:
    f.write("# место фрагмента: пункт, раздел (текст вне пунктов) или «-» (шапка) <TAB> фрагмент;"
            " файл создаёт tools/grammar/spans.py из spec/nulix.md\n")
    for place, s in rows:
        f.write(f"{place}\t{s}\n")
print("фрагментов:", len(rows), "уникальных:", len(set(s for _, s in rows)))
