from pathlib import Path


def matching_paren(text: str, open_index: int) -> int:
    depth = 0
    for index in range(open_index, len(text)):
        char = text[index]
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return index
    raise SystemExit("unclosed call")


def argument_count(body: str) -> int:
    depth = 0
    parts = []
    start = 0
    for index, char in enumerate(body):
        if char in "([{":
            depth += 1
        elif char in ")]}":
            depth -= 1
        elif char == "," and depth == 0:
            parts.append(body[start:index])
            start = index + 1
    parts.append(body[start:])
    return sum(1 for part in parts if part.strip())


def add_missing_argument(text: str, needle: str, old_arg_count: int, argument: str) -> str:
    cursor = 0
    while True:
        start = text.find(needle, cursor)
        if start == -1:
            return text
        open_index = start + len(needle) - 1
        close_index = matching_paren(text, open_index)
        body = text[open_index + 1 : close_index]
        if argument_count(body) == old_arg_count:
            stripped = body.rstrip()
            trailing = body[len(stripped) :]
            if stripped.endswith(","):
                line_start = text.rfind("\n", 0, close_index) + 1
                close_indent = text[line_start:close_index]
                addition = f"\n{close_indent}    {argument},"
                new_body = stripped + addition + trailing
            else:
                new_body = stripped + f", {argument}" + trailing
            text = text[: open_index + 1] + new_body + text[close_index:]
            cursor = open_index + 1 + len(new_body) + 1
        else:
            cursor = close_index + 1


path = Path("crates/dawn-sector/src/node/transit_flow.rs")
text = path.read_text()
text = add_missing_argument(text, ".complete_outgoing_transit(", 3, "Tick::ZERO")
text = add_missing_argument(text, ".import_transit(", 4, "Tick::ZERO")
path.write_text(text)
