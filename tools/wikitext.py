"""Minimal MediaWiki template parser.

Only what this project needs: split a page into templates, and split a template
body into positional / named parameters while respecting nested `{{...}}`,
`[[...]]` and `{|...|}` constructs and piped wiki links.
"""

from __future__ import annotations

import re
from typing import Dict, List, Tuple


def _split_top_level(body: str, sep: str = "|") -> List[str]:
    parts: List[str] = []
    depth_curly = 0
    depth_link = 0
    cur: List[str] = []
    i = 0
    while i < len(body):
        ch = body[i]
        two = body[i : i + 2]
        if two == "{{":
            depth_curly += 1
            cur.append(two)
            i += 2
            continue
        if two == "}}":
            depth_curly = max(0, depth_curly - 1)
            cur.append(two)
            i += 2
            continue
        if two == "[[":
            depth_link += 1
            cur.append(two)
            i += 2
            continue
        if two == "]]":
            depth_link = max(0, depth_link - 1)
            cur.append(two)
            i += 2
            continue
        if ch == sep and depth_curly == 0 and depth_link == 0:
            parts.append("".join(cur))
            cur = []
            i += 1
            continue
        cur.append(ch)
        i += 1
    parts.append("".join(cur))
    return parts


class Template:
    __slots__ = ("name", "positional", "named", "raw")

    def __init__(self, name: str, positional: List[str], named: Dict[str, str], raw: str):
        self.name = name
        self.positional = positional
        self.named = named
        self.raw = raw

    def get(self, key: str, default: str = "") -> str:
        if key in self.named:
            return self.named[key]
        try:
            idx = int(key)
        except ValueError:
            return default
        if 0 <= idx < len(self.positional):
            return self.positional[idx]
        return default

    def __repr__(self) -> str:  # pragma: no cover - debugging aid
        return f"<Template {self.name} named={list(self.named)} positional={len(self.positional)}>"


_COMMENT_RE = re.compile(r"<!--.*?-->", re.S)


def parse_template(raw: str) -> Template:
    body = raw
    if body.startswith("{{"):
        body = body[2:]
    if body.endswith("}}"):
        body = body[:-2]
    body = _COMMENT_RE.sub("", body)
    chunks = _split_top_level(body)
    name = chunks[0].strip()
    positional: List[str] = []
    named: Dict[str, str] = {}
    for chunk in chunks[1:]:
        eq = _find_top_level_eq(chunk)
        if eq == -1:
            positional.append(chunk.strip())
        else:
            key = chunk[:eq].strip()
            val = chunk[eq + 1 :].strip()
            named[key] = val
    return Template(name, positional, named, raw)


def _find_top_level_eq(chunk: str) -> int:
    depth_curly = 0
    depth_link = 0
    i = 0
    while i < len(chunk):
        two = chunk[i : i + 2]
        if two == "{{":
            depth_curly += 1
            i += 2
            continue
        if two == "}}":
            depth_curly -= 1
            i += 2
            continue
        if two == "[[":
            depth_link += 1
            i += 2
            continue
        if two == "]]":
            depth_link -= 1
            i += 2
            continue
        if chunk[i] == "=" and depth_curly <= 0 and depth_link <= 0:
            return i
        i += 1
    return -1


def iter_templates(text: str) -> List[Tuple[int, int, Template]]:
    """Return every top-level template as (start, end, template)."""
    out: List[Tuple[int, int, Template]] = []
    i = 0
    while i < len(text):
        if text.startswith("{{", i):
            j = _match_template(text, i)
            if j == -1:
                i += 2
                continue
            raw = text[i:j]
            out.append((i, j, parse_template(raw)))
            i = j
        else:
            i += 1
    return out


def _match_template(text: str, start: int) -> int:
    depth = 0
    i = start
    while i < len(text):
        if text.startswith("{{", i):
            depth += 1
            i += 2
            continue
        if text.startswith("}}", i):
            depth -= 1
            i += 2
            if depth == 0:
                return i
            continue
        i += 1
    return -1


def find_template(text: str, name: str) -> Template | None:
    for _, _, tpl in iter_templates(text):
        if tpl.name == name:
            return tpl
    return None


def find_templates(text: str, name: str) -> List[Template]:
    return [tpl for _, _, tpl in iter_templates(text) if tpl.name == name]


def strip_markup(text: str) -> str:
    """Turn wiki markup into readable plain text, keeping status names in []."""
    out: List[str] = []
    i = 0
    while i < len(text):
        if text.startswith("{{", i):
            j = _match_template(text, i)
            if j == -1:
                out.append(text[i])
                i += 1
                continue
            tpl = parse_template(text[i:j])
            if tpl.name in ("StatusEffect", "StatusEffectIcon"):
                out.append("[" + tpl.get("0") + "]")
            elif tpl.name in ("SkillCon", "Keyword"):
                out.append("[" + strip_markup(tpl.get("0")) + "]")
            elif tpl.name == "SkillHint":
                # Keep the inner text (it already carries [Status] markers);
                # wrapping it again would nest brackets and break parsing.
                out.append(strip_markup(tpl.get("0")))
            elif tpl.name in ("Icons", "Icon"):
                out.append(tpl.get("0"))
            elif tpl.name in ("GiftTT",):
                out.append(tpl.get("0"))
            elif tpl.name in ("Color", "Colour"):
                out.append(strip_markup(tpl.get("1")))
            else:
                inner = tpl.positional[-1] if tpl.positional else ""
                out.append(strip_markup(inner))
            i = j
            continue
        if text.startswith("[[", i):
            j = text.find("]]", i)
            if j == -1:
                out.append(text[i])
                i += 1
                continue
            inner = text[i + 2 : j]
            out.append(strip_markup(inner.split("|")[-1]))
            i = j + 2
            continue
        if text.startswith("<br", i):
            j = text.find(">", i)
            if j == -1:
                out.append(text[i])
                i += 1
                continue
            out.append("\n")
            i = j + 1
            continue
        if text.startswith("<", i):
            j = text.find(">", i)
            if j == -1:
                out.append(text[i])
                i += 1
                continue
            i = j + 1
            continue
        if text.startswith("'''", i) or text.startswith("''", i):
            i += 3 if text.startswith("'''", i) else 2
            continue
        out.append(text[i])
        i += 1
    lines = [ln.strip(" \t") for ln in "".join(out).split("\n")]
    return "\n".join(ln for ln in lines if ln != "")
