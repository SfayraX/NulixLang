"""Собирает документ о грамматике: шаблон + текст грамматики как есть + итоги проверки."""
import hashlib, os, re, subprocess, sys
import lark
G, OUT = sys.argv[1], sys.argv[2]
grammar = open(os.path.join(G, "nulix.lark"), encoding="utf-8").read().rstrip("\n")
tpl = open(os.path.join(G, "doc-template.md"), encoding="utf-8").read()
res = subprocess.run([sys.executable, os.path.join(os.path.dirname(__file__), "check.py"), G],
                     capture_output=True, text=True)
lines = [l for l in res.stdout.splitlines() if l.strip()]
assert res.returncode == 0 and not any(("НЕ " in l) or ("ОШИБОЧНО" in l) for l in lines), res.stdout
get = lambda key: next(l for l in lines if l.startswith(key))
m_g = re.search(r"правил (\d+), ключевых слов (\d+), знаков и пометок (\d+)", get("грамматика"))
m_f = re.search(r"разобрано (\d+) из (\d+); пропущено с причиной (\d+)", get("фрагменты"))
m_s = re.search(r"разных фрагментов (\d+); одиночных лексем (\d+) \(ключевых слов (\d+)\); не учтено (\d+)", get("раздел"))
m_p = re.search(r"разобрано (\d+) из (\d+)", get("тестовые"))
m_n = re.search(r"отвергнуто (\d+) из (\d+)", get("отрицательные"))
extra = sum(1 for l in open(os.path.join(G, "fragments.tsv"), encoding="utf-8") if l.startswith("+ "))


def golden(name):
    """Эталонный файл сверки: число принятых и отвергнутых текстов и его первая строка."""
    rows = open(os.path.join(G, name), encoding="utf-8").read().splitlines()
    ok = sum(1 for r in rows[1:] if r.split("\t")[3] == "OK")
    return ok, len(rows) - 1 - ok, rows[0]


t_ok, t_bad, _ = golden("trees.golden.tsv")
f_ok, f_bad, f_head = golden("fuzz.golden.tsv")
d_ok, d_bad, _ = golden("depth.golden.tsv")
m_c = re.search(r"раскрытий правил в принятых текстах (\d+) из (\d+)", f_head)
assert m_c.group(1) == m_c.group(2), f_head
words = sorted(set(re.findall(r'"([a-z]+)"', grammar)))
assert len(words) == int(m_g.group(2)), (len(words), m_g.group(2))
check = "\n".join([
    f"- **Конфликтов нет.** Генератор разборщиков Lark {lark.__version__} собрал грамматику как LALR(1) в строгом режиме, где любой конфликт — ошибка: {m_g.group(1)} правил, {m_g.group(2)} ключевое слово, {m_g.group(3)} знак и пометка. Три заведомо неоднозначные правки грамматики тот же режим отверг, то есть проверка работает.",
    f"- **Фрагменты спецификации.** В спецификации нет многострочных примеров кода, только фрагменты в тексте. В разделе «Синтаксис» их {m_s.group(1)} разных: {m_s.group(2)} — одиночные лексемы, остальные учтены поштучно, неучтённых {m_s.group(4)}. Вместе с фрагментами из других разделов и {extra} примерами к принятым решениям разобрано {m_f.group(1)} из {m_f.group(2)}; {m_f.group(3)} пропущены с причиной: названия команд, отдельные знаки, `++` и `--`, которых в языке нет, и `asm(…)`, чьи операнды — открытый вопрос. Неполные фрагменты дополнялись до целой конструкции, например `for x in xs` разбирался как `for x in xs {{ }}`.".replace("{{ }}", "{ }"),
    f"- **Тестовые программы.** {m_p.group(1)} из {m_p.group(2)} разобраны целиком. Их составил Claude из конструкций спецификации; это не код прогонов на примерах — тот в спецификацию не входил.",
    f"- **Отрицательные примеры.** {m_n.group(1)} из {m_n.group(2)} отвергнуты, как и должны: `a and b or c`, `a < b < c`, `a << b + c`, `x++`, `007`, `else` с новой строки и другие.",
    f"- **Разборщик компилятора.** Рукописный разборщик на Rust сверяется с этой грамматикой тестами `cargo test` в папке `compiler`: на каждом тексте он обязан принять или отвергнуть его так же, как Lark, и построить такое же дерево. Тексты — {t_ok + t_bad} примеров, перечисленных выше ({t_ok} принимаются, {t_bad} отвергаются), и {f_ok + f_bad} текстов, выведенных из правил грамматики ({f_ok} и {f_bad}): среди них есть принятый текст на каждое из {m_c.group(2)} раскрытий правил, а отвергаемые получены одной порчей — лишней, пропущенной или переставленной лексемой или частью. Отдельно сверяются {d_ok + d_bad} текстов у предела вложенности: на 255 и 256 уровнях ({d_ok} текстов) и на 257 ({d_bad}).",
    "- **Источник.** Фрагменты выписаны из `spec/nulix.md` в `grammar/spans.tsv`, у каждого — номер пункта. Список создаёт `tools/grammar/spans.py`; проверка на GitHub пересобирает его и сверяет с лежащим в репозитории. До 21 ноября 2026 список снимался с выгрузки рабочего документа, а фрагменты для проверки были взяты из его ревизии 243.",
])
doc = (tpl.replace("{{CHECK}}", check)
          .replace("{{KEYWORDS}}", " ".join(f"`{w}`" for w in words))
          .replace("{{GRAMMAR}}", grammar))
assert "{{" not in doc.replace("{{ }}", "")
open(OUT, "w", encoding="utf-8").write(doc)
block = doc.split("```\n", 1)[1].rsplit("\n```", 1)[0]
print("грамматика в документе совпадает с проверенным файлом:", block == grammar)
print("sha256 грамматики:", hashlib.sha256((grammar + "\n").encode()).hexdigest()[:16], " строк в документе:", doc.count("\n") + 1)
