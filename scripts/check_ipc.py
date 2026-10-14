#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""IPC contract check: frontend `invoke('cmd', {...})` payload keys must be
snake_case and match the Rust `#[tauri::command]` parameter names.

Run:  python3 scripts/check_ipc.py   (also `npm run check:ipc`)
Exit 0 = clean; exit 1 = mismatch with file:line details.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src"
RS = ROOT / "src-tauri" / "src"
MAIN_RS = RS / "main.rs"

CAMEL = re.compile(r"[a-z]+[A-Z][A-Za-z0-9]*")


def rust_commands():
    cmds = {}
    for f in RS.rglob("*.rs"):
        text = f.read_text(errors="ignore")
        for am in re.finditer(r"#\[tauri::command(?:\([^)]*\))?\]", text):
            fm = re.search(
                r"pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+(\w+)\s*\((.*?)\)",
                text[am.end():am.end() + 3000],
                re.S,
            )
            if not fm:
                continue
            fn, params = fm.group(1), fm.group(2)
            names = set()
            for pm in re.finditer(r"(\w+)\s*:\s*[^,)]+", params):
                n = pm.group(1)
                if n not in ("app", "state"):
                    names.add(n)
            # Keep first (real) definition; later dupes (tests) must agree.
            cmds.setdefault(fn, names)
    # Cross-check with the generate_handler! list in main.rs (source of truth
    # for what the frontend can actually call).
    handler = set()
    main = MAIN_RS.read_text(errors="ignore")
    hm = re.search(r"generate_handler!\[(.*?)\]", main, re.S)
    if hm:
        for part in hm.group(1).split(","):
            part = part.strip()
            if "::" in part:
                handler.add(part.split("::")[-1].split()[0])
            elif part and not part.startswith("//"):
                handler.add(part.split()[0])
    return cmds, handler


def extract_invoke_payloads(text):
    """Yield (cmd, keys, line_no) for each invoke('cmd', {..}) call,
    parsing balanced braces from the call site only."""
    out = []
    for m in re.finditer(r"invoke[^\n;()]*\(\s*['\"]([a-z][a-z_0-9]+)['\"]", text):
        cmd = m.group(1)
        line_no = text[: m.start()].count("\n") + 1
        # Find the first '{' after the command within the call parens.
        i = m.end()
        depth_paren = 1
        brace_start = -1
        j = i
        while j < len(text) and depth_paren > 0:
            c = text[j]
            if c == "(":
                depth_paren += 1
            elif c == ")":
                depth_paren -= 1
                if depth_paren == 0:
                    break
            elif c == "{" and brace_start == -1:
                brace_start = j
                break
            j += 1
        keys = set()
        if brace_start != -1:
            depth = 0
            k = brace_start
            in_str = None
            while k < len(text):
                c = text[k]
                if in_str:
                    if c == in_str and text[k - 1] != "\\":
                        in_str = None
                elif c in ("'", '"', "`"):
                    in_str = c
                elif c == "{":
                    depth += 1
                elif c == "}":
                    depth -= 1
                    if depth == 0:
                        break
                elif c == ")" and depth == 0:
                    break
                k += 1
            body = text[brace_start:k]
            # Top-level keys only: identifiers followed by ':' at depth 1.
            d = 0
            instr = None
            buf = ""
            for ch in body:
                if instr:
                    if ch == instr:
                        instr = None
                    buf = ""
                    continue
                if ch in ("'", '"', "`"):
                    instr = ch
                    buf = ""
                    continue
                if ch == "{":
                    d += 1
                    buf = ""
                    continue
                if ch == "}":
                    d -= 1
                    buf = ""
                    continue
                if d == 1:
                    if ch == ":":
                        key = buf.strip().split()[-1] if buf.strip() else ""
                        if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key or ""):
                            keys.add(key)
                        buf = ""
                    elif ch == "," or ch == "\n":
                        buf = ""
                    else:
                        buf += ch
        out.append((cmd, keys, line_no))
    return out


def main() -> int:
    cmds, handler = rust_commands()
    errors = []
    total = 0
    for p in list(SRC.rglob("*.ts")) + list(SRC.rglob("*.tsx")):
        text = p.read_text(errors="ignore")
        rel = str(p.relative_to(ROOT))
        for cmd, keys, ln in extract_invoke_payloads(text):
            total += 1
            if cmd not in cmds and cmd not in handler:
                errors.append(f"{rel}:{ln}: unknown command '{cmd}'")
                continue
            for k in keys:
                if CAMEL.search(k):
                    want = sorted(cmds.get(cmd, set()))
                    errors.append(
                        f"{rel}:{ln}: camelCase key '{k}' for '{cmd}' "
                        f"(expects snake_case {want})"
                    )
    if errors:
        print("IPC CONTRACT FAILURES:")
        for e in errors:
            print("  " + e)
        print(f"\n{len(errors)} problem(s).")
        return 1
    print(f"IPC OK: {len(cmds)} commands, {total} invoke sites checked.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
