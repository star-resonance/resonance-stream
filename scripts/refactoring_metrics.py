#!/usr/bin/env python3
"""Structural metrics of one checkout of resonance-stream. Run the same file on each commit.

  python3 metrics.py <worktree>  -> JSON on stdout

Function metrics are a text estimate, not a parser: comments and string literals are blanked, a `fn` is found by
its signature and its body by brace matching, and anything after a `#[cfg(test)]` line in a file (the test module)
is left out. Complexity = 1 + `if` + `while` + `for` + `loop` + `&&` + `||` + match arms (`=>`). It will differ
from clippy's cognitive complexity; it is the same estimate on both commits.
"""
import json, re, sys
from pathlib import Path

root = Path(sys.argv[1])
AREAS = {
    "core": "crates/core/src", "core_tests": "crates/core/tests", "types": "crates/types/src",
    "llama": "crates/llama/src", "ui": "src", "app": "src-tauri/src",
}
DATA_FILES = {"kanji_on_table.rs"}  # a table, not logic


def rs_files(rel):
    base = root / rel
    return sorted(p for p in base.rglob("*.rs")) if base.exists() else []


def blank(text):
    """Replace comments, string and char literals with spaces (newlines kept) so counting sees code only."""
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        two = text[i:i + 2]
        if two == "//":
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i)); i = j
        elif two == "/*":
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append(re.sub(r"[^\n]", " ", text[i:j])); i = j
        elif c == "r" and re.match(r'r#*"', text[i:i + 8]):
            m = re.match(r'r(#*)"', text[i:])
            end = '"' + m.group(1)
            j = text.find(end, i + len(m.group(0)))
            j = n if j < 0 else j + len(end)
            out.append(re.sub(r"[^\n]", " ", text[i:j])); i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            j = min(n, j + 1)
            out.append(re.sub(r"[^\n]", " ", text[i:j])); i = j
        elif c == "'":
            m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:i + 12])
            if m:
                out.append(" " * len(m.group(0))); i += len(m.group(0))
            else:
                out.append(c); i += 1
        else:
            out.append(c); i += 1
    return "".join(out)


FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(<[^{;]*?>)?\s*\(")


def functions(path):
    raw = path.read_text(encoding="utf-8", errors="replace")
    cut = raw.find("#[cfg(test)]")
    if cut >= 0:
        raw = raw[:cut]
    code = blank(raw)
    found = []
    for m in FN.finditer(code):
        # parameters: balanced parentheses from the `(`
        i = m.end() - 1
        depth, j = 0, i
        while j < len(code):
            if code[j] in "([{":
                depth += 1
            elif code[j] in ")]}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        params_text = code[i + 1:j]
        # split on top-level commas
        d, parts, cur = 0, [], ""
        for ch in params_text:
            if ch in "([{<":
                d += 1
            elif ch in ")]}>":
                d -= 1 if not (ch == ">" and cur.endswith("-")) else 0
            if ch == "," and d == 0:
                parts.append(cur); cur = ""
            else:
                cur += ch
        if cur.strip():
            parts.append(cur)
        parts = [p for p in parts if p.strip() and not re.match(r"\s*&?\s*(mut\s+)?self\b", p)]
        # body: first `{` or `;` after the parameters
        k = j + 1
        while k < len(code) and code[k] not in "{;":
            k += 1
        if k >= len(code) or code[k] == ";":
            continue  # a declaration with no body
        depth, e = 0, k
        while e < len(code):
            if code[e] == "{":
                depth += 1
            elif code[e] == "}":
                depth -= 1
                if depth == 0:
                    break
            e += 1
        body = code[k:e + 1]
        start_line = code.count("\n", 0, m.start()) + 1
        end_line = code.count("\n", 0, e) + 1
        cx = 1 + sum(len(re.findall(p, body)) for p in (r"\bif\b", r"\bwhile\b", r"\bfor\b", r"\bloop\b", r"&&", r"\|\|", r"=>"))
        found.append({"name": m.group(1), "file": str(path.relative_to(root)), "lines": end_line - start_line + 1,
                      "complexity": cx, "params": len(parts)})
    return found


def count_lines(files):
    return sum(len(p.read_text(encoding="utf-8", errors="replace").splitlines()) for p in files)


out = {"lines": {}, "files": {}, "fn": {}}
all_fn = []
big = []
for area, rel in AREAS.items():
    files = rs_files(rel)
    out["lines"][area] = count_lines(files)
    out["files"][area] = len(files)
    if area != "core_tests":
        for p in files:
            n = len(p.read_text(encoding="utf-8", errors="replace").splitlines())
            big.append((n, str(p.relative_to(root))))
            if p.name not in DATA_FILES:
                all_fn += functions(p)
out["lines"]["rust_total"] = sum(v for k, v in out["lines"].items())
out["biggest_files"] = [f"{n} {p}" for n, p in sorted(big, reverse=True)[:6]]
out["fn"] = {
    "count": len(all_fn),
    "lines_ge_80": sum(f["lines"] >= 80 for f in all_fn),
    "lines_ge_150": sum(f["lines"] >= 150 for f in all_fn),
    "complexity_ge_15": sum(f["complexity"] >= 15 for f in all_fn),
    "complexity_ge_20": sum(f["complexity"] >= 20 for f in all_fn),
    "params_ge_7": sum(f["params"] >= 7 for f in all_fn),
    "longest": sorted(all_fn, key=lambda f: -f["lines"])[:6],
    "most_complex": sorted(all_fn, key=lambda f: -f["complexity"])[:6],
}

# test attributes per area
def count(pattern, rel, exclude_tail=False):
    total = 0
    for p in rs_files(rel):
        text = p.read_text(encoding="utf-8", errors="replace")
        total += len(re.findall(pattern, text))
    return total

out["test_attrs"] = {a: count(r"#\[test\]", r) for a, r in AREAS.items()}
out["test_attrs"]["total"] = sum(out["test_attrs"].values())

# production-code panics: .unwrap() / .expect( before the test module of each file
def panics(rel):
    u = e = 0
    for p in rs_files(rel):
        raw = p.read_text(encoding="utf-8", errors="replace")
        cut = raw.find("#[cfg(test)]")
        code = blank(raw[:cut] if cut >= 0 else raw)
        u += len(re.findall(r"\.unwrap\(\)", code))
        e += len(re.findall(r"\.expect\(", code))
    return {"unwrap": u, "expect": e}

out["panic_sites"] = {a: panics(r) for a, r in AREAS.items() if a not in ("core_tests",)}

# structure facts
def grep_count(pattern, rels):
    return sum(count(pattern, r) for r in rels)

out["facts"] = {
    "AppConfig_struct_definitions": grep_count(r"pub struct AppConfig\b", ["crates", "src", "src-tauri/src"]),
    "default_tab_limit_definitions": grep_count(r"fn default_tab_limits?\b|fn default_channel_limit\b|fn default_limit", ["crates", "src", "src-tauri/src"]),
}
wf = sorted(p.name for p in (root / ".github/workflows").glob("*.yml"))
out["workflows"] = {"count": len(wf), "names": wf}
sh = sorted(p.name for p in (root / ".github/scripts").glob("*.test.sh")) if (root / ".github/scripts").exists() else []
out["shell_test_scripts"] = len(sh)
mem = root / "MEMORY.md"
# MEMORY.md is gone since D-32; older commits still have it.
out["memory_index"] = (
    {"lines": len(mem.read_text(encoding="utf-8").splitlines()), "bytes": mem.stat().st_size} if mem.exists() else None
)
docs = sorted(p.name for p in (root / "docs").glob("*.md")) if (root / "docs").exists() else []
out["docs"] = {"count": len(docs), "names": docs}
snap = list((root / "crates/core/tests/snapshots").glob("*.snap")) if (root / "crates/core/tests/snapshots").exists() else []
out["golden_snapshots"] = len(snap)

# capabilities (webview permissions) and plugin dependencies
caps = []
cdir = root / "src-tauri/capabilities"
if cdir.exists():
    for p in cdir.glob("*.json"):
        try:
            caps += json.loads(p.read_text(encoding="utf-8")).get("permissions", [])
        except Exception:
            pass
out["webview_permissions"] = {"count": len(caps), "shell_or_opener": [c for c in caps if isinstance(c, str) and (c.startswith("shell:") or c.startswith("opener:"))]}
cargo = (root / "src-tauri/Cargo.toml").read_text(encoding="utf-8")
out["shell_plugin_dependency"] = "tauri-plugin-shell" in cargo

# runbook (python) and its real-app rows
py = [p for p in (root / "runbook").rglob("*.py") if "node_modules" not in str(p)] if (root / "runbook").exists() else []
out["runbook"] = {
    "python_lines": sum(len(p.read_text(encoding="utf-8", errors="replace").splitlines()) for p in py),
    "pipelines": len(list((root / "runbook/runbook/pipelines").glob("*.py"))) - 1 if (root / "runbook/runbook/pipelines").exists() else 0,
    "real_app_rows": sum(len(re.findall(r"\.auto\(", p.read_text(encoding="utf-8", errors="replace"))) for p in (root / "runbook/runbook/pipelines").glob("*.py")) if (root / "runbook/runbook/pipelines").exists() else 0,
    "python_tests": sum(len(re.findall(r"^def test_", p.read_text(encoding="utf-8", errors="replace"), re.M)) for p in (root / "runbook/tests").glob("test_*.py")) if (root / "runbook/tests").exists() else 0,
}
print(json.dumps(out, indent=2, ensure_ascii=False))
