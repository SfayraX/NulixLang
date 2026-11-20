"""Номера разделов и пунктов спецификации.

Номер пункта — номер раздела и номер пункта в нём: 9.14. Номера не меняются и повторно
не используются: новый пункт получает следующий свободный номер своего раздела, где бы ни
стоял, новый раздел — следующий свободный номер раздела.

  python3 tools/spec/numbers.py check spec/nulix.md   — проверить номера
  python3 tools/spec/numbers.py strip spec/nulix.md   — текст без номеров (для сверки)
"""
import re
import sys

SECTION = re.compile(r"^## (\d+)\. (.+)$")
# пункт списка: «- **9.14.** текст»; у открытого вопроса — «- [ ] **16.3.** текст»
ITEM = re.compile(r"^- (\[[ x]\] )?\*\*(\d+)\.(\d+)\.\*\* (.+)$")


def strip(text):
    """Текст без номеров разделов и пунктов."""
    out = []
    for line in text.split("\n"):
        m = SECTION.match(line)
        if m:
            line = "## " + m.group(2)
        m = ITEM.match(line)
        if m:
            line = "- " + (m.group(1) or "") + m.group(4)
        out.append(line)
    return "\n".join(out)


def check(text):
    """Ошибки нумерации: раздел без номера, пункт без номера или с чужим номером раздела,
    повтор номера."""
    errors, sections, items = [], set(), set()
    current = None
    in_code = False
    for n, line in enumerate(text.split("\n"), 1):
        if line.startswith("```"):
            in_code = not in_code
        if in_code:
            continue
        if line.startswith("## "):
            m = SECTION.match(line)
            if not m:
                errors.append(f"строка {n}: раздел без номера: {line}")
                current = None
                continue
            current = int(m.group(1))
            if current in sections:
                errors.append(f"строка {n}: номер раздела {current} повторён")
            sections.add(current)
        elif line.startswith("- "):
            m = ITEM.match(line)
            if not m:
                errors.append(f"строка {n}: пункт без номера: {line[:60]}")
                continue
            sec, k = int(m.group(2)), int(m.group(3))
            if sec != current:
                errors.append(f"строка {n}: пункт {sec}.{k} стоит в разделе {current}")
            if (sec, k) in items:
                errors.append(f"строка {n}: номер {sec}.{k} повторён")
            items.add((sec, k))
    return errors, len(sections), len(items)


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in ("check", "strip"):
        print(__doc__)
        sys.exit(2)
    text = open(sys.argv[2], encoding="utf-8").read()
    if sys.argv[1] == "strip":
        sys.stdout.write(strip(text))
        sys.exit(0)
    errors, ns, ni = check(text)
    for e in errors:
        print(e)
    print(f"разделов {ns}, пунктов {ni}, ошибок {len(errors)}")
    sys.exit(1 if errors else 0)
